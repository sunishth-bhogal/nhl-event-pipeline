//! Breakaway exchange server: real people trading the "home wins" contract on
//! a replayed NHL game, alongside the market-making bot.
//!
//!     cargo run --release --bin server -- --port 8080 --speed 10
//!
//! Architecture
//! * One thread per connection parses HTTP and hands requests to a single
//!   engine thread over a channel. The engine thread owns the exchange, so
//!   every command is applied in one global order (the sequencer), exactly as
//!   the journal records it.
//! * Group commit: the engine thread takes a batch of requests, appends them
//!   to the write-ahead journal, makes the journal durable with one fsync, and
//!   only then replies. An acknowledged order survives a crash.
//! * On start it replays the journal of the market in progress, so a crash or
//!   restart resumes the same game with the same book, accounts and clock.
//! * Market data streams to browsers as Server-Sent Events, ten times a second.
//! * The game clock replays a featured game at `--speed` times real time; when
//!   it ends the market settles and, after a short pause, the next game starts.
//!
//! API (JSON)
//!   POST   /api/join          -> {token, account, cash}       $1,000 play money
//!   POST   /api/orders        {side:"buy"|"sell", price:1-99, qty, tif:"gtc"|"ioc"}
//!   DELETE /api/orders/<id>
//!   GET    /api/me            balances, shares, open orders, marked P&L
//!   GET    /api/market        current market
//!   GET    /api/state         state hash + journal length (for recovery checks)
//!   GET    /api/stream        Server-Sent Events: book, trades, clock
//! Authenticated calls send `Authorization: Bearer <token>`.

mod http;

use breakaway_engine::bot::MarketMaker;
use breakaway_engine::exchange::{ExCommand, ExEvent, Exchange, RiskReject};
use breakaway_engine::journal::{Entry, Journal};
use breakaway_engine::{AccountId, CancelReason, Event, OrderId, Reject, Side, TimeInForce};
use http::*;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MM: AccountId = 0;
const NOISE: std::ops::Range<u32> = 1..21;
const FIRST_USER: AccountId = 100;
const START_CASH: i64 = 100_000; // $1,000 of play money per player per game
const MM_CASH: i64 = 5_000_000;
const NOISE_CASH: i64 = 100_000;
const MM_DELAY: usize = 3; // game seconds the bot lags the true fair price
const TICK: Duration = Duration::from_millis(50);
const BROADCAST_EVERY: Duration = Duration::from_millis(100);
const ORDERS_PER_SEC: f64 = 20.0; // per player, default (--rate)
const MAX_CONNECTIONS: usize = 512;

#[derive(Clone)]
struct Config {
    host: String,
    port: u16,
    web: PathBuf,
    data: PathBuf,
    fair: PathBuf,
    speed: f64,
    pause_after_final: Duration,
    /// orders per second per player; 0 disables the limit (load tests only)
    rate: f64,
}

type Reply = Sender<(u16, String)>;

enum Req {
    Join { reply: Reply },
    Place { token: String, side: Side, price: u8, qty: u32, tif: TimeInForce, reply: Reply },
    Cancel { token: String, order_id: OrderId, reply: Reply },
    Me { token: String, reply: Reply },
    Market { reply: Reply },
    State { reply: Reply },
    Subscribe { tx: Sender<String> },
}

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
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * self.unit()).cos()
    }
}

fn random_token() -> u64 {
    let mut b = [0u8; 8];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_ok() {
        return u64::from_le_bytes(b);
    }
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;
    Rng(t | 1).next()
}

// ---------------------------------------------------------------- games
struct Game {
    id: String,
    fair: Vec<f64>,
    home_won: bool,
}

fn games(cfg: &Config) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(&cfg.fair)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "csv")).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn load_game(path: &Path) -> Game {
    let text = std::fs::read_to_string(path).expect("read fair csv");
    let mut lines = text.lines();
    let home_won = lines.next().unwrap_or("").ends_with('1');
    let fair = lines.filter_map(|l| l.split(',').nth(1)).map(|v| v.parse().unwrap()).collect();
    Game { id: path.file_stem().unwrap().to_string_lossy().into(), fair, home_won }
}

// ---------------------------------------------------------------- market
struct Market {
    game_index: usize,
    g: Game,
    x: Exchange,
    j: Journal,
    mm: MarketMaker,
    rng: Rng,
    sec: usize,
    sec_f: f64,
    settled_at: Option<Instant>,
    tokens: HashMap<u64, AccountId>,
    next_account: AccountId,
    noise_orders: Vec<(OrderId, AccountId, usize)>,
    new_trades: Vec<(usize, u8, u32, bool)>,
    last_price: i32,
    volume: u64,
    rate: HashMap<AccountId, (f64, Instant)>,
    rate_limit: f64,
    scratch: Vec<ExEvent>,
}

impl Market {
    fn end(&self) -> usize {
        self.g.fair.len().saturating_sub(1)
    }

    fn start(cfg: &Config, game_index: usize) -> Market {
        let list = games(cfg);
        assert!(!list.is_empty(), "no games in {}", cfg.fair.display());
        let game_index = game_index % list.len();
        let g = load_game(&list[game_index]);
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
        let name = format!("{}-{stamp}.journal", g.id);
        let (j, x, _) = Journal::open(&cfg.data.join(&name)).expect("open journal");
        let seed = random_token() | 1;
        let mut m = Market::assemble(game_index, g, x, j, seed);
        m.exec(ExCommand::Deposit { account: MM, cents: MM_CASH });
        for a in NOISE {
            m.exec(ExCommand::Deposit { account: a, cents: NOISE_CASH });
        }
        m.append(Entry::Clock(0));
        m.j.commit().expect("journal commit");
        write_pointer(cfg, game_index, &name);
        eprintln!("market {} started ({})", m.g.id, name);
        m
    }

    fn recover(cfg: &Config) -> Option<Market> {
        let text = std::fs::read_to_string(cfg.data.join("current")).ok()?;
        let (idx, name) = text.trim().split_once(' ')?;
        let list = games(cfg);
        let game_index: usize = idx.parse().ok()?;
        let g = load_game(list.get(game_index)?);
        let t0 = Instant::now();
        let (j, x, clock, joins) = Journal::open_with_joins(&cfg.data.join(name)).expect("replay journal");
        x.check().expect("recovered exchange is consistent");
        let entries = j.entries;
        let mut m = Market::assemble(game_index, g, x, j, random_token() | 1);
        m.sec = clock.unwrap_or(0) as usize;
        m.sec_f = m.sec as f64;
        for (account, token) in joins {
            m.tokens.insert(token, account);
            m.next_account = m.next_account.max(account + 1);
        }
        for (_, id, owner, _, _) in m.x.engine().snapshot() {
            if owner == MM {
                m.mm.track(id);
            } else if NOISE.contains(&owner) {
                m.noise_orders.push((id, owner, m.sec + 60));
            }
        }
        if m.x.settled().is_some() {
            m.settled_at = Some(Instant::now());
        }
        eprintln!("recovered market {} from {name}: {entries} journal entries in {:.1} ms, game second {}", m.g.id, t0.elapsed().as_secs_f64() * 1e3, m.sec);
        Some(m)
    }

    fn assemble(game_index: usize, g: Game, x: Exchange, j: Journal, seed: u64) -> Market {
        Market {
            game_index,
            g,
            x,
            j,
            mm: MarketMaker::new(MM),
            rng: Rng(seed),
            sec: 0,
            sec_f: 0.0,
            settled_at: None,
            tokens: HashMap::new(),
            next_account: FIRST_USER,
            noise_orders: Vec::new(),
            new_trades: Vec::new(),
            last_price: -1,
            volume: 0,
            rate: HashMap::new(),
            rate_limit: ORDERS_PER_SEC,
            scratch: Vec::with_capacity(64),
        }
    }

    fn append(&mut self, e: Entry) {
        // Without the journal we can't promise durability, so stop rather than continue.
        self.j.append(&e).expect("journal append failed");
    }

    /// Journal, then apply. Returns this command's events.
    fn exec(&mut self, cmd: ExCommand) -> Vec<ExEvent> {
        self.append(Entry::Cmd(cmd));
        self.scratch.clear();
        let mut out = std::mem::take(&mut self.scratch);
        self.x.apply(&cmd, &mut out);
        for ev in &out {
            if let ExEvent::Book(Event::Fill { price, qty, taker_side, .. }) = *ev {
                self.new_trades.push((self.sec, price, qty, taker_side == Side::Buy));
                self.last_price = price as i32;
                self.volume += qty as u64;
            }
        }
        let evs = out.clone();
        self.scratch = out;
        evs
    }

    fn fair_now(&self) -> f64 {
        self.g.fair[self.sec.min(self.end())]
    }

    // ---- clock, bot and background traders
    fn tick(&mut self, dt: f64, speed: f64) {
        if self.settled_at.is_some() {
            return;
        }
        self.sec_f += dt * speed;
        while (self.sec as f64) < self.sec_f.floor() && self.sec < self.end() {
            self.sec += 1;
            self.append(Entry::Clock(self.sec as u32));
            self.step_second();
        }
        if self.sec >= self.end() {
            let home_won = self.g.home_won;
            self.exec(ExCommand::Settle { home_won });
            self.settled_at = Some(Instant::now());
            eprintln!("market {} settled: home {}", self.g.id, if home_won { "won" } else { "lost" });
        }
    }

    fn step_second(&mut self) {
        let s = self.sec;
        let f = self.fair_now();
        if s % 2 == 0 {
            let seen = self.g.fair[s.saturating_sub(MM_DELAY)];
            let past = self.g.fair[s.saturating_sub(MM_DELAY + 20)];
            for cmd in self.mm.plan(&self.x, seen, past) {
                let evs = self.exec(cmd);
                if let (ExCommand::Place { .. }, Some(ExEvent::Book(Event::Accepted { order_id, .. }))) = (cmd, evs.first()) {
                    if self.x.engine().order(*order_id).is_some() {
                        self.mm.track(*order_id);
                    }
                }
            }
        }
        // a little background flow so the market is alive between human orders
        if self.rng.unit() < 0.3 {
            let acct = NOISE.start + self.rng.below(NOISE.len() as u64) as u32;
            let guess = (f + self.rng.normal() * 6.0).clamp(1.0, 99.0);
            let qty = 1 + self.rng.below(12) as u32;
            let (bb, ba) = (self.x.engine().best_bid(), self.x.engine().best_ask());
            let cmd = match (bb, ba) {
                (_, Some(a)) if guess > a as f64 + 1.0 => ExCommand::Place { account: acct, side: Side::Buy, price: a, qty, tif: TimeInForce::Ioc },
                (Some(b), _) if guess < b as f64 - 1.0 => ExCommand::Place { account: acct, side: Side::Sell, price: b, qty, tif: TimeInForce::Ioc },
                _ => {
                    let side = if guess >= f { Side::Buy } else { Side::Sell };
                    let p = if side == Side::Buy { guess.floor() } else { guess.ceil() }.clamp(1.0, 99.0) as u8;
                    ExCommand::Place { account: acct, side, price: p, qty, tif: TimeInForce::Gtc }
                }
            };
            let evs = self.exec(cmd);
            if let Some(ExEvent::Book(Event::Accepted { order_id, .. })) = evs.first() {
                if self.x.engine().order(*order_id).is_some() {
                    let until = s + 60 + self.rng.below(240) as usize;
                    self.noise_orders.push((*order_id, acct, until));
                }
            }
        }
        let mut i = 0;
        while i < self.noise_orders.len() {
            let (oid, acct, until) = self.noise_orders[i];
            if self.x.engine().order(oid).is_none() {
                self.noise_orders.swap_remove(i);
            } else if s >= until {
                self.exec(ExCommand::Cancel { account: acct, order_id: oid });
                self.noise_orders.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    // ---- requests
    fn auth(&self, token: &str) -> Option<AccountId> {
        let t = u64::from_str_radix(token, 16).ok()?;
        self.tokens.get(&t).copied()
    }

    fn allow(&mut self, account: AccountId, limit: f64) -> bool {
        if limit <= 0.0 {
            return true;
        }
        let now = Instant::now();
        let e = self.rate.entry(account).or_insert((limit, now));
        e.0 = (e.0 + now.duration_since(e.1).as_secs_f64() * limit).min(limit);
        e.1 = now;
        if e.0 >= 1.0 {
            e.0 -= 1.0;
            true
        } else {
            false
        }
    }

    fn handle(&mut self, r: Req, replies: &mut Vec<(Reply, u16, String)>, subs: &mut Vec<Sender<String>>) {
        match r {
            Req::Join { reply } => {
                if self.settled_at.is_some() {
                    replies.push((reply, 409, err("this game is over; the next one starts shortly")));
                    return;
                }
                let account = self.next_account;
                self.next_account += 1;
                let token = random_token();
                self.tokens.insert(token, account);
                self.append(Entry::Join { account, token });
                self.exec(ExCommand::Deposit { account, cents: START_CASH });
                replies.push((reply, 201, format!("{{\"token\":\"{token:016x}\",\"account\":{account},\"market\":\"{}\",\"cash\":{START_CASH}}}", self.g.id)));
            }
            Req::Place { token, side, price, qty, tif, reply } => {
                let Some(account) = self.auth(&token) else {
                    replies.push((reply, 401, err("unknown token for this game; join again")));
                    return;
                };
                if !self.allow(account, self.rate_limit) {
                    replies.push((reply, 429, err("slow down: too many orders a second")));
                    return;
                }
                let evs = self.exec(ExCommand::Place { account, side, price, qty, tif });
                let code = if evs.iter().any(|e| matches!(e, ExEvent::RiskRejected { .. } | ExEvent::Book(Event::Rejected { .. }))) { 422 } else { 200 };
                replies.push((reply, code, events_json(&evs)));
            }
            Req::Cancel { token, order_id, reply } => {
                let Some(account) = self.auth(&token) else {
                    replies.push((reply, 401, err("unknown token for this game; join again")));
                    return;
                };
                let evs = self.exec(ExCommand::Cancel { account, order_id });
                let code = if evs.iter().any(|e| matches!(e, ExEvent::Book(Event::Rejected { .. }))) { 404 } else { 200 };
                replies.push((reply, code, events_json(&evs)));
            }
            Req::Me { token, reply } => match self.auth(&token) {
                None => replies.push((reply, 401, err("unknown token for this game; join again"))),
                Some(account) => replies.push((reply, 200, self.me_json(account))),
            },
            Req::Market { reply } => replies.push((reply, 200, self.market_json())),
            Req::State { reply } => {
                let check = match self.x.check() {
                    Ok(()) => "\"ok\"".to_string(),
                    Err(e) => quote(&e),
                };
                let body = format!(
                    "{{\"market\":\"{}\",\"state_hash\":\"{:016x}\",\"journal_entries\":{},\"sec\":{},\"escrow\":{},\"deposits\":{},\"check\":{check}}}",
                    self.g.id,
                    self.x.engine().state_hash(),
                    self.j.entries,
                    self.sec,
                    self.x.escrow(),
                    self.x.deposits()
                );
                replies.push((reply, 200, body));
            }
            Req::Subscribe { tx } => {
                let _ = tx.send(self.market_json_typed("market"));
                let _ = tx.send(self.tick_json());
                subs.push(tx);
            }
        }
    }

    fn status(&self) -> &'static str {
        if self.settled_at.is_some() { "final" } else { "live" }
    }

    fn market_json_typed(&self, kind: &str) -> String {
        format!(
            "{{\"type\":\"{kind}\",\"market\":\"{}\",\"sec\":{},\"end\":{},\"status\":\"{}\",\"home_won\":{}}}",
            self.g.id,
            self.sec,
            self.end(),
            self.status(),
            match self.x.settled() {
                Some(b) => b.to_string(),
                None => "null".into(),
            }
        )
    }

    fn market_json(&self) -> String {
        self.market_json_typed("market")
    }

    fn tick_json(&mut self) -> String {
        let e = self.x.engine();
        let lv = |side| e.depth(side, 8).iter().map(|(p, q, _)| format!("[{p},{q}]")).collect::<Vec<_>>().join(",");
        let mut trades = String::new();
        for (i, (s, p, q, buy)) in self.new_trades.iter().rev().take(40).rev().enumerate() {
            let _ = write!(trades, "{}[{s},{p},{q},{}]", if i > 0 { "," } else { "" }, *buy as u8);
        }
        let msg = format!(
            "{{\"type\":\"tick\",\"market\":\"{}\",\"sec\":{},\"status\":\"{}\",\"fair\":{:.2},\"last\":{},\"volume\":{},\"bids\":[{}],\"asks\":[{}],\"trades\":[{trades}]}}",
            self.g.id,
            self.sec,
            self.status(),
            self.fair_now(),
            self.last_price,
            self.volume,
            lv(Side::Buy),
            lv(Side::Sell)
        );
        self.new_trades.clear();
        msg
    }

    fn me_json(&self, account: AccountId) -> String {
        let a = *self.x.account(account).unwrap();
        let f = self.fair_now();
        let marked = a.total_cash() as f64 + a.yes_shares() as f64 * f + a.no_shares() as f64 * (100.0 - f) - START_CASH as f64;
        let mut orders = String::new();
        for (side, id, owner, price, qty) in self.x.engine().snapshot() {
            if owner == account {
                let _ = write!(
                    orders,
                    "{}{{\"id\":{id},\"side\":\"{}\",\"price\":{price},\"qty\":{qty}}}",
                    if orders.is_empty() { "" } else { "," },
                    if side == Side::Buy { "buy" } else { "sell" }
                );
            }
        }
        format!(
            "{{\"account\":{account},\"market\":\"{}\",\"cash\":{},\"cash_reserved\":{},\"yes\":{},\"no\":{},\"pnl_marked\":{:.0},\"orders\":[{orders}]}}",
            self.g.id,
            a.cash,
            a.cash_reserved,
            a.yes_shares(),
            a.no_shares(),
            marked
        )
    }
}

fn write_pointer(cfg: &Config, idx: usize, name: &str) {
    let tmp = cfg.data.join("current.tmp");
    std::fs::write(&tmp, format!("{idx} {name}\n")).expect("write pointer");
    std::fs::rename(&tmp, cfg.data.join("current")).expect("swap pointer");
}

fn err(msg: &str) -> String {
    format!("{{\"error\":{}}}", quote(msg))
}

fn events_json(evs: &[ExEvent]) -> String {
    let mut s = String::from("{\"events\":[");
    for (i, e) in evs.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = match *e {
            ExEvent::Book(Event::Accepted { order_id, side, price, qty, .. }) => write!(s, "{{\"type\":\"accepted\",\"order_id\":{order_id},\"side\":\"{}\",\"price\":{price},\"qty\":{qty}}}", side_str(side)),
            ExEvent::Book(Event::Fill { taker, maker, price, qty, taker_side, .. }) => write!(s, "{{\"type\":\"fill\",\"taker\":{taker},\"maker\":{maker},\"price\":{price},\"qty\":{qty},\"taker_side\":\"{}\"}}", side_str(taker_side)),
            ExEvent::Book(Event::Rested { order_id, qty }) => write!(s, "{{\"type\":\"rested\",\"order_id\":{order_id},\"qty\":{qty}}}"),
            ExEvent::Book(Event::Cancelled { order_id, qty, reason }) => write!(s, "{{\"type\":\"cancelled\",\"order_id\":{order_id},\"qty\":{qty},\"reason\":\"{}\"}}", match reason {
                CancelReason::Requested => "requested",
                CancelReason::IocRemainder => "ioc_remainder",
                CancelReason::SelfTrade => "self_trade",
            }),
            ExEvent::Book(Event::Rejected { reason }) => write!(s, "{{\"type\":\"rejected\",\"reason\":\"{}\"}}", match reason {
                Reject::BadPrice => "price must be 1-99",
                Reject::BadQty => "quantity must be 1-1,000,000",
                Reject::UnknownOrder => "no such open order",
                Reject::NotOwner => "not your order",
            }),
            ExEvent::RiskRejected { reason, .. } => write!(s, "{{\"type\":\"rejected\",\"reason\":\"{}\"}}", match reason {
                RiskReject::InsufficientFunds => "not enough cash to cover this order if it fully fills",
                RiskReject::MarketSettled => "this game is over",
                RiskReject::UnknownAccount => "unknown account",
                RiskReject::BadDeposit => "bad deposit",
            }),
            ExEvent::Deposited { cents, .. } => write!(s, "{{\"type\":\"deposited\",\"cents\":{cents}}}"),
            ExEvent::Settled { home_won, .. } => write!(s, "{{\"type\":\"settled\",\"home_won\":{home_won}}}"),
        };
    }
    s.push_str("]}");
    s
}

fn side_str(s: Side) -> &'static str {
    if s == Side::Buy { "buy" } else { "sell" }
}

// ---------------------------------------------------------------- engine thread
fn engine_loop(cfg: Config, rx: Receiver<Req>) {
    let mut market = Market::recover(&cfg).unwrap_or_else(|| Market::start(&cfg, 0));
    market.rate_limit = cfg.rate;
    let mut subs: Vec<Sender<String>> = Vec::new();
    let mut batch: Vec<Req> = Vec::with_capacity(4096);
    let mut replies: Vec<(Reply, u16, String)> = Vec::with_capacity(4096);
    let mut last_tick = Instant::now();
    let mut last_broadcast = Instant::now();
    loop {
        match rx.recv_timeout(TICK.saturating_sub(last_tick.elapsed())) {
            Ok(r) => batch.push(r),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while batch.len() < 4096 {
            match rx.try_recv() {
                Ok(r) => batch.push(r),
                Err(_) => break,
            }
        }
        let before = market.j.entries;
        for r in batch.drain(..) {
            market.handle(r, &mut replies, &mut subs);
        }
        if last_tick.elapsed() >= TICK {
            let dt = last_tick.elapsed().as_secs_f64();
            last_tick = Instant::now();
            market.tick(dt, cfg.speed);
            if market.settled_at.map_or(false, |t| t.elapsed() >= cfg.pause_after_final) {
                market.j.commit().expect("journal commit");
                market = Market::start(&cfg, market.game_index + 1);
                market.rate_limit = cfg.rate;
                let msg = market.market_json_typed("market");
                subs.retain(|s| s.send(msg.clone()).is_ok());
            }
        }
        // group commit: everything in this batch is durable before anyone hears back
        if market.j.entries != before {
            market.j.commit().expect("journal commit");
        }
        for (tx, code, body) in replies.drain(..) {
            let _ = tx.send((code, body));
        }
        if last_broadcast.elapsed() >= BROADCAST_EVERY && !subs.is_empty() {
            last_broadcast = Instant::now();
            let msg = market.tick_json();
            subs.retain(|s| s.send(msg.clone()).is_ok());
        }
    }
}

// ---------------------------------------------------------------- connections
fn call(tx: &Sender<Req>, make: impl FnOnce(Reply) -> Req) -> (u16, String) {
    let (rtx, rrx) = mpsc::channel();
    if tx.send(make(rtx)).is_err() {
        return (503, err("exchange unavailable"));
    }
    rrx.recv_timeout(Duration::from_secs(5)).unwrap_or((503, err("exchange busy, try again")))
}

fn parse_order(body: &[u8]) -> Result<(Side, u8, u32, TimeInForce), &'static str> {
    let m = parse_flat(body).ok_or("body must be a flat JSON object")?;
    let side = match m.get("side").map(|s| s.as_str()) {
        Some("buy") => Side::Buy,
        Some("sell") => Side::Sell,
        _ => return Err("side must be \"buy\" or \"sell\""),
    };
    let price: u8 = m.get("price").and_then(|v| v.parse().ok()).ok_or("price must be a whole number of cents, 1-99")?;
    let qty: u32 = m.get("qty").and_then(|v| v.parse().ok()).ok_or("qty must be a positive whole number")?;
    let tif = match m.get("tif").map(|s| s.as_str()) {
        None | Some("gtc") => TimeInForce::Gtc,
        Some("ioc") => TimeInForce::Ioc,
        _ => return Err("tif must be \"gtc\" or \"ioc\""),
    };
    Ok((side, price, qty, tif))
}

/// Returns false when the connection should close.
fn route(req: &Request, w: &mut TcpStream, tx: &Sender<Req>, web: &Path, ka: bool) -> std::io::Result<bool> {
    let token = req.bearer().unwrap_or("").to_string();
    let reply = |w: &mut TcpStream, (code, body): (u16, String)| json(w, code, &body, ka).map(|_| ka);
    match (req.method.as_str(), req.path.as_str()) {
        ("POST", "/api/join") => reply(w, call(tx, |reply| Req::Join { reply })),
        ("POST", "/api/orders") => match parse_order(&req.body) {
            Ok((side, price, qty, tif)) => reply(w, call(tx, |reply| Req::Place { token, side, price, qty, tif, reply })),
            Err(msg) => error(w, 400, msg, ka).map(|_| ka),
        },
        ("DELETE", p) if p.starts_with("/api/orders/") => match p["/api/orders/".len()..].parse::<OrderId>() {
            Ok(order_id) => reply(w, call(tx, |reply| Req::Cancel { token, order_id, reply })),
            Err(_) => error(w, 400, "order id must be a number", ka).map(|_| ka),
        },
        ("GET", "/api/me") => reply(w, call(tx, |reply| Req::Me { token, reply })),
        ("GET", "/api/market") => reply(w, call(tx, |reply| Req::Market { reply })),
        ("GET", "/api/state") => reply(w, call(tx, |reply| Req::State { reply })),
        ("GET", "/api/stream") => {
            stream_events(w, tx)?;
            Ok(false)
        }
        ("GET", p) if !p.starts_with("/api/") => match static_path(web, p) {
            Some(file) => {
                let body = std::fs::read(&file)?;
                respond(w, 200, content_type(&file), &body, ka).map(|_| ka)
            }
            None => error(w, 404, "not found", ka).map(|_| ka),
        },
        (_, p) if p.starts_with("/api/") => error(w, 405, "method not allowed", ka).map(|_| ka),
        _ => error(w, 405, "method not allowed", ka).map(|_| ka),
    }
}

fn stream_events(w: &mut TcpStream, tx: &Sender<Req>) -> std::io::Result<()> {
    w.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: keep-alive\r\nX-Accel-Buffering: no\r\n\r\n")?;
    w.set_write_timeout(Some(Duration::from_secs(10)))?;
    let (stx, srx) = mpsc::channel::<String>();
    if tx.send(Req::Subscribe { tx: stx }).is_err() {
        return Ok(());
    }
    loop {
        match srx.recv_timeout(Duration::from_secs(15)) {
            Ok(msg) => {
                // coalesce a backlog into the newest messages
                let mut buf = format!("data: {msg}\n\n");
                while let Ok(m) = srx.try_recv() {
                    buf.push_str("data: ");
                    buf.push_str(&m);
                    buf.push_str("\n\n");
                }
                w.write_all(buf.as_bytes())?;
            }
            Err(RecvTimeoutError::Timeout) => w.write_all(b": keep-alive\n\n")?,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn serve(stream: TcpStream, tx: Sender<Req>, web: Arc<PathBuf>, conns: Arc<AtomicUsize>) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    if let Ok(read_half) = stream.try_clone() {
        let mut r = BufReader::new(read_half);
        let mut w = stream;
        loop {
            let req = match read_request(&mut r) {
                Ok(req) => req,
                Err(ReadError::Bad(msg, code)) => {
                    let _ = error(&mut w, code, msg, false);
                    break;
                }
                Err(_) => break,
            };
            let ka = req.keep_alive();
            match route(&req, &mut w, &tx, &web, ka) {
                Ok(true) => continue,
                _ => break,
            }
        }
    }
    conns.fetch_sub(1, Ordering::SeqCst);
}

fn parse_args() -> Config {
    let mut cfg = Config {
        host: "127.0.0.1".into(),
        port: 8080,
        web: PathBuf::from("../web"),
        data: PathBuf::from("data/server"),
        fair: PathBuf::from("data/fair"),
        speed: 10.0,
        pause_after_final: Duration::from_secs(20),
        rate: ORDERS_PER_SEC,
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--host" => cfg.host = v,
            "--port" => cfg.port = v.parse().expect("--port"),
            "--web" => cfg.web = v.into(),
            "--data" => cfg.data = v.into(),
            "--fair" => cfg.fair = v.into(),
            "--speed" => cfg.speed = v.parse().expect("--speed"),
            "--pause" => cfg.pause_after_final = Duration::from_secs_f64(v.parse().expect("--pause")),
            "--rate" => cfg.rate = v.parse().expect("--rate"),
            other => panic!("unknown argument {other}"),
        }
        i += 2;
    }
    cfg
}

fn main() {
    let cfg = parse_args();
    std::fs::create_dir_all(&cfg.data).expect("create data dir");
    let listener = TcpListener::bind((cfg.host.as_str(), cfg.port)).expect("bind");
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel::<Req>();
    let engine_cfg = cfg.clone();
    let engine = std::thread::Builder::new().name("engine".into()).spawn(move || engine_loop(engine_cfg, rx)).unwrap();
    // wait until the market is up (and any journal replayed) before accepting traffic
    let (code, _) = call(&tx, |reply| Req::Market { reply });
    assert_eq!(code, 200, "engine failed to start");
    println!("listening on http://{addr}");
    std::io::stdout().flush().ok();

    let web = Arc::new(cfg.web.clone());
    let conns = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        if engine.is_finished() {
            eprintln!("engine thread stopped; shutting down");
            std::process::exit(1);
        }
        let Ok(mut stream) = stream else { continue };
        if conns.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            conns.fetch_sub(1, Ordering::SeqCst);
            let _ = error(&mut stream, 503, "server is full, try again", false);
            continue;
        }
        let (tx, web, conns) = (tx.clone(), web.clone(), conns.clone());
        std::thread::spawn(move || serve(stream, tx, web, conns));
    }
}
