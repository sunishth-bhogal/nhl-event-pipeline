//! Simulated market for a replayed game, run through the real exchange.
//!
//!     cargo run --release --bin simulate -- data/fair ../web/markets
//!
//! For every data/fair/<id>.csv (second-by-second fair price from the
//! win-probability model) this runs one market and writes <id>.json for the
//! replay page. Seeded by the game id, so every run is identical.
//!
//! Participants
//! Snapshots (quotes and book) are taken every 5 game seconds, together, so
//! the page's bid/ask always matches its order book.
//!
//! * Market maker (account 0): sees the fair price 3 seconds late. Quotes two
//!   levels a side, widens its spread when the price has been jumping, and
//!   shades its quotes against its inventory.
//! * Sharp traders: see the true fair price immediately and take any quote
//!   that is off by more than 2c, which is how the market maker loses money
//!   right after a goal.
//! * Noise traders: trade on a rough guess (fair +/- about 6c). If the guess is
//!   through a quote they take it; otherwise they post their own limit order,
//!   which they cancel after a while.
//! Every order goes through risk checks, matching and escrow, and the market
//! settles at the final whistle.

use breakaway_engine::exchange::{Account, ExCommand, ExEvent, Exchange};
use breakaway_engine::*;
use std::fmt::Write as _;
use std::fs;

const MM: AccountId = 0;
const SHARPS: std::ops::Range<u32> = 1..11;
const NOISE: std::ops::Range<u32> = 11..71;
const MM_DELAY: usize = 3;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn normal(&mut self) -> f64 {
        let u = self.unit().max(1e-12);
        let v = self.unit();
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

struct Tape {
    quotes: String, // [sec,bid,ask,last]
    book: String,   // [sec,[[p,q]..],[[p,q]..]]
    trades: String, // [sec,price,qty,aggressor(1 buy,0 sell),maker_is_mm]
    mm: String,     // [sec,inventory,pnl_mark]
    n_trades: u64,
    volume: u64,
    last: i32,
}

fn mark(a: &Account, fair: f64, deposit: i64) -> f64 {
    a.total_cash() as f64 + a.yes_shares() as f64 * fair + a.no_shares() as f64 * (100.0 - fair) - deposit as f64
}

fn simulate(id: &str, home_won: bool, fair: &[f64]) -> String {
    let seed = id.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    let mut rng = Rng(seed | 1);
    let mut x = Exchange::new();
    let mut out: Vec<ExEvent> = Vec::with_capacity(256);
    let mm_deposit: i64 = 2_000_000; // $20,000
    let trader_deposit: i64 = 50_000; // $500 each
    x.apply(&ExCommand::Deposit { account: MM, cents: mm_deposit }, &mut out);
    for a in SHARPS.chain(NOISE) {
        x.apply(&ExCommand::Deposit { account: a, cents: trader_deposit }, &mut out);
    }
    let deposits_traders = |range: std::ops::Range<u32>| range.len() as i64 * trader_deposit;

    let mut mm_orders: Vec<OrderId> = Vec::new();
    let mut resting: Vec<(OrderId, AccountId, usize)> = Vec::new(); // noise orders + expiry second
    let mut tape = Tape { quotes: String::new(), book: String::new(), trades: String::new(), mm: String::new(), n_trades: 0, volume: 0, last: -1 };
    let mut last_quote = (0u8, 0u8, 0i64);
    let mut rejected = 0u64;

    let send = |x: &mut Exchange, cmd: ExCommand, sec: usize, out: &mut Vec<ExEvent>, tape: &mut Tape, rejected: &mut u64| -> Option<OrderId> {
        out.clear();
        x.apply(&cmd, out);
        let mut id = None;
        for ev in out.iter() {
            match *ev {
                ExEvent::Book(Event::Accepted { order_id, .. }) => id = Some(order_id),
                ExEvent::Book(Event::Fill { price, qty, taker_side, maker_account, .. }) => {
                    tape.n_trades += 1;
                    tape.volume += qty as u64;
                    tape.last = price as i32;
                    let _ = write!(tape.trades, "[{sec},{price},{qty},{},{}],", (taker_side == Side::Buy) as u8, (maker_account == MM) as u8);
                }
                ExEvent::RiskRejected { .. } => *rejected += 1,
                _ => {}
            }
        }
        id
    };

    for sec in 0..fair.len() {
        let f = fair[sec];
        // ---- market maker: requote every 2 s on what it saw MM_DELAY seconds ago
        if sec % 2 == 0 {
            let seen = fair[sec.saturating_sub(MM_DELAY)];
            let past = fair[sec.saturating_sub(MM_DELAY + 20)];
            let vol = (seen - past).abs();
            let a = *x.account(MM).unwrap();
            let inventory = a.yes_shares() - a.no_shares();
            let half = (1.5 + vol * 0.6).min(12.0);
            let skew = (inventory as f64 / 40.0).clamp(-6.0, 6.0);
            let bid = (seen - half - skew).floor().clamp(1.0, 98.0) as u8;
            let ask = ((seen + half - skew).ceil().clamp(2.0, 99.0) as u8).max(bid + 1);
            if (bid, ask, inventory) != last_quote {
                for oid in mm_orders.drain(..) {
                    send(&mut x, ExCommand::Cancel { account: MM, order_id: oid }, sec, &mut out, &mut tape, &mut rejected);
                }
                // stop quoting once the game is all but decided
                if seen > 1.0 && seen < 99.0 {
                    for (p, q, s) in [(bid, 40, Side::Buy), (bid.saturating_sub(3).max(1), 80, Side::Buy), (ask, 40, Side::Sell), ((ask + 3).min(99), 80, Side::Sell)] {
                        let cmd = ExCommand::Place { account: MM, side: s, price: p, qty: q, tif: TimeInForce::Gtc };
                        if let Some(oid) = send(&mut x, cmd, sec, &mut out, &mut tape, &mut rejected) {
                            if x.engine().order(oid).is_some() {
                                mm_orders.push(oid);
                            }
                        }
                    }
                }
                last_quote = (bid, ask, inventory);
            }
        }

        // ---- sharp traders: act on the true price, immediately
        let sharp_tries = if rng.unit() < 0.35 { 1 } else { 0 } + if (f - fair[sec.saturating_sub(1)]).abs() > 3.0 { 3 } else { 0 };
        for _ in 0..sharp_tries {
            let acct = SHARPS.start + rng.below(SHARPS.len() as u64) as u32;
            let (bb, ba) = (x.engine().best_bid(), x.engine().best_ask());
            let qty = 5 + rng.below(25) as u32;
            if let Some(a) = ba {
                if f - a as f64 > 2.0 {
                    let limit = (f - 1.0).floor().clamp(1.0, 99.0) as u8;
                    send(&mut x, ExCommand::Place { account: acct, side: Side::Buy, price: limit, qty, tif: TimeInForce::Ioc }, sec, &mut out, &mut tape, &mut rejected);
                    continue;
                }
            }
            if let Some(b) = bb {
                if b as f64 - f > 2.0 {
                    let limit = (f + 1.0).ceil().clamp(1.0, 99.0) as u8;
                    send(&mut x, ExCommand::Place { account: acct, side: Side::Sell, price: limit, qty, tif: TimeInForce::Ioc }, sec, &mut out, &mut tape, &mut rejected);
                }
            }
        }

        // ---- noise traders: rough guesses, take or post
        if rng.unit() < 0.45 {
            let acct = NOISE.start + rng.below(NOISE.len() as u64) as u32;
            let guess = (f + rng.normal() * 6.0).clamp(1.0, 99.0);
            let qty = 1 + rng.below(15) as u32;
            let (bb, ba) = (x.engine().best_bid(), x.engine().best_ask());
            let cmd = match (bb, ba) {
                (_, Some(a)) if guess > a as f64 + 1.0 => ExCommand::Place { account: acct, side: Side::Buy, price: a, qty, tif: TimeInForce::Ioc },
                (Some(b), _) if guess < b as f64 - 1.0 => ExCommand::Place { account: acct, side: Side::Sell, price: b, qty, tif: TimeInForce::Ioc },
                _ => {
                    let side = if guess >= f { Side::Buy } else { Side::Sell };
                    let p = match side {
                        Side::Buy => guess.floor(),
                        Side::Sell => guess.ceil(),
                    }
                    .clamp(1.0, 99.0) as u8;
                    ExCommand::Place { account: acct, side, price: p, qty, tif: TimeInForce::Gtc }
                }
            };
            if let Some(oid) = send(&mut x, cmd, sec, &mut out, &mut tape, &mut rejected) {
                if x.engine().order(oid).is_some() {
                    resting.push((oid, acct, sec + 60 + rng.below(240) as usize));
                }
            }
        }
        // noise orders expire
        let mut i = 0;
        while i < resting.len() {
            let (oid, acct, until) = resting[i];
            if x.engine().order(oid).is_none() {
                resting.swap_remove(i);
            } else if sec >= until {
                send(&mut x, ExCommand::Cancel { account: acct, order_id: oid }, sec, &mut out, &mut tape, &mut rejected);
                resting.swap_remove(i);
            } else {
                i += 1;
            }
        }

        // ---- tape
        if sec % 5 == 0 || sec + 1 == fair.len() {
            let (bb, ba) = (x.engine().best_bid(), x.engine().best_ask());
            let _ = write!(tape.quotes, "[{sec},{},{},{}],", bb.map_or(-1, |v| v as i32), ba.map_or(-1, |v| v as i32), tape.last);
        }
        if sec % 5 == 0 || sec + 1 == fair.len() {
            let fmt = |lv: Vec<(u8, u64, u32)>| lv.iter().map(|(p, q, _)| format!("[{p},{q}]")).collect::<Vec<_>>().join(",");
            let _ = write!(tape.book, "[{sec},[{}],[{}]],", fmt(x.engine().depth(Side::Buy, 6)), fmt(x.engine().depth(Side::Sell, 6)));
        }
        if sec % 30 == 0 || sec + 1 == fair.len() {
            let a = x.account(MM).unwrap();
            let _ = write!(tape.mm, "[{sec},{},{:.0}],", a.yes_shares() - a.no_shares(), mark(a, f, mm_deposit));
        }
    }

    let end_sec = fair.len().saturating_sub(1);
    let mm_inventory_at_end = {
        let a = x.account(MM).unwrap();
        a.yes_shares() - a.no_shares()
    };
    out.clear();
    x.apply(&ExCommand::Settle { home_won }, &mut out);
    x.check().expect("exchange invariants after settlement");
    let pnl = |range: std::ops::Range<u32>, dep: i64| -> i64 { range.map(|a| x.account(a).unwrap().total_cash()).sum::<i64>() - dep };
    let mm_pnl = x.account(MM).unwrap().total_cash() - mm_deposit;
    let sharp_pnl = pnl(SHARPS, deposits_traders(SHARPS));
    let noise_pnl = pnl(NOISE, deposits_traders(NOISE));
    assert_eq!(mm_pnl + sharp_pnl + noise_pnl, 0, "zero-sum after settlement");

    let trim = |s: &mut String| {
        if s.ends_with(',') {
            s.pop();
        }
    };
    trim(&mut tape.quotes);
    trim(&mut tape.book);
    trim(&mut tape.trades);
    trim(&mut tape.mm);
    format!(
        "{{\"id\":{id},\"home_won\":{home_won},\"end\":{end_sec},\"summary\":{{\"trades\":{},\"contracts\":{},\"risk_rejections\":{rejected},\"mm_inventory_at_end\":{mm_inventory_at_end},\"pnl_cents\":{{\"market_maker\":{mm_pnl},\"sharps\":{sharp_pnl},\"noise\":{noise_pnl}}}}},\"quotes\":[{}],\"book\":[{}],\"trades\":[{}],\"mm\":[{}]}}",
        tape.n_trades, tape.volume, tape.quotes, tape.book, tape.trades, tape.mm
    )
}

fn main() {
    let mut args = std::env::args().skip(1);
    let fair_dir = args.next().unwrap_or_else(|| "data/fair".into());
    let out_dir = args.next().unwrap_or_else(|| "../web/markets".into());
    fs::create_dir_all(&out_dir).expect("create output dir");
    let mut files: Vec<_> = fs::read_dir(&fair_dir).expect("fair dir").filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "csv")).collect();
    files.sort();
    for path in files {
        let id = path.file_stem().unwrap().to_string_lossy().to_string();
        let text = fs::read_to_string(&path).expect("read csv");
        let mut lines = text.lines();
        let home_won = lines.next().unwrap().ends_with('1');
        let fair: Vec<f64> = lines.filter_map(|l| l.split(',').nth(1)).map(|v| v.parse().unwrap()).collect();
        let t0 = std::time::Instant::now();
        let json = simulate(&id, home_won, &fair);
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        fs::write(format!("{out_dir}/{id}.json"), &json).expect("write");
        let s = json.split("\"summary\":").nth(1).unwrap().split(",\"quotes\"").next().unwrap();
        println!("{id}: {s} ({ms:.0} ms, {} KB)", json.len() / 1024);
    }
}
