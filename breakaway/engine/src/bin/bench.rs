//! Throughput and latency benchmark for the matching engine.
//!
//!     cargo run --release --bin bench [commands]      (default 5,000,000)
//!
//! Order flow: a fair price drifts like a live win probability. Traders post
//! limit orders around it, some cross the spread (half of those immediate-or-
//! cancel), and about a third of all commands cancel a resting order. The
//! stream is recorded once, then a fresh engine replays it, so random-number
//! generation is not timed. The engine is single-threaded by design.

use breakaway_engine::exchange::{ExCommand, Exchange};
use breakaway_engine::*;
use std::time::Instant;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn record(n: usize) -> (Vec<Command>, Engine) {
    let mut rng = Rng(0xB5EA_CAFE_1234_5678);
    let mut e = Engine::with_capacity(1 << 16);
    let mut out = Vec::with_capacity(64);
    let mut live: Vec<(OrderId, AccountId)> = Vec::with_capacity(1 << 16);
    let mut cmds = Vec::with_capacity(n);
    let mut mid: i64 = 50;
    for i in 0..n {
        if i % 64 == 0 {
            mid = (mid + rng.below(3) as i64 - 1).clamp(8, 92);
        }
        let account = rng.below(1000) as AccountId;
        let r = rng.below(100);
        let cmd = if r < 33 && !live.is_empty() {
            let k = rng.below(live.len() as u64) as usize;
            let (id, owner) = live.swap_remove(k);
            Command::Cancel { account: owner, order_id: id }
        } else {
            let side = if rng.below(2) == 0 { Side::Buy } else { Side::Sell };
            let crossing = r >= 88; // ~12% of commands take liquidity
            let off = if crossing { rng.below(3) as i64 } else { 1 + rng.below(8) as i64 };
            let price = match (side, crossing) {
                (Side::Buy, false) | (Side::Sell, true) => mid - off,
                (Side::Buy, true) | (Side::Sell, false) => mid + off,
            }
            .clamp(1, 99) as u8;
            let tif = if crossing && rng.below(2) == 0 { TimeInForce::Ioc } else { TimeInForce::Gtc };
            Command::Place { account, side, price, qty: 1 + rng.below(50) as u32, tif }
        };
        out.clear();
        e.apply(&cmd, &mut out);
        for ev in &out {
            match *ev {
                Event::Rested { order_id, .. } => {
                    if let Command::Place { account, .. } = cmd {
                        live.push((order_id, account));
                    }
                }
                Event::Fill { maker, .. } => {
                    if e.order(maker).is_none() {
                        if let Some(p) = live.iter().position(|x| x.0 == maker) {
                            live.swap_remove(p);
                        }
                    }
                }
                Event::Cancelled { order_id, reason: CancelReason::SelfTrade, .. } => {
                    if let Some(p) = live.iter().position(|x| x.0 == order_id) {
                        live.swap_remove(p);
                    }
                }
                _ => {}
            }
        }
        cmds.push(cmd);
    }
    (cmds, e)
}

fn pct(sorted: &[u32], p: f64) -> u32 {
    sorted[((sorted.len() as f64 - 1.0) * p).round() as usize]
}

fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("model name")).map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string()))
        .unwrap_or_else(|| "unknown CPU".into())
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    eprintln!("recording {n} commands...");
    let (cmds, reference) = record(n);

    let mut kinds = [0usize; 3]; // place-gtc, place-ioc, cancel
    for c in &cmds {
        match c {
            Command::Place { tif: TimeInForce::Gtc, .. } => kinds[0] += 1,
            Command::Place { tif: TimeInForce::Ioc, .. } => kinds[1] += 1,
            Command::Cancel { .. } => kinds[2] += 1,
        }
    }

    // ---- throughput: best of 3 replays
    let mut best = f64::MAX;
    let mut fills = 0usize;
    let mut final_hash = 0u64;
    for _ in 0..3 {
        let mut e = Engine::with_capacity(1 << 16);
        let mut out = Vec::with_capacity(64);
        let t0 = Instant::now();
        let mut f = 0usize;
        for c in &cmds {
            out.clear();
            e.apply(c, &mut out);
            f += out.iter().filter(|x| matches!(x, Event::Fill { .. })).count();
        }
        let dt = t0.elapsed().as_secs_f64();
        best = best.min(dt);
        fills = f;
        final_hash = e.state_hash();
    }
    assert_eq!(final_hash, reference.state_hash(), "replay must rebuild the recorded book");

    // ---- latency: time every command individually
    let mut timer_cost = Vec::with_capacity(100_000);
    for _ in 0..100_000 {
        let a = Instant::now();
        let b = Instant::now();
        timer_cost.push((b - a).as_nanos() as u32);
    }
    timer_cost.sort_unstable();
    let overhead = pct(&timer_cost, 0.5);

    let mut e = Engine::with_capacity(1 << 16);
    let mut out = Vec::with_capacity(64);
    let mut lat = Vec::with_capacity(cmds.len());
    for c in &cmds {
        out.clear();
        let a = Instant::now();
        e.apply(c, &mut out);
        let ns = (a.elapsed().as_nanos() as u32).saturating_sub(overhead);
        lat.push(ns);
    }
    lat.sort_unstable();

    // ---- the same flow through the full exchange: accounts, risk checks, escrow
    let mut ex_cmds: Vec<ExCommand> = (0..1000).map(|a| ExCommand::Deposit { account: a, cents: 1_000_000_000 }).collect();
    ex_cmds.extend(cmds.iter().map(|c| match *c {
        Command::Place { account, side, price, qty, tif } => ExCommand::Place { account, side, price, qty, tif },
        Command::Cancel { account, order_id } => ExCommand::Cancel { account, order_id },
    }));
    let mut ex_best = f64::MAX;
    for _ in 0..3 {
        let mut x = Exchange::new();
        let mut out = Vec::with_capacity(64);
        let t0 = Instant::now();
        for c in &ex_cmds {
            out.clear();
            x.apply(c, &mut out);
        }
        ex_best = ex_best.min(t0.elapsed().as_secs_f64());
        x.check().expect("exchange invariants after benchmark");
        assert_eq!(x.engine().state_hash(), reference.state_hash(), "risk layer must not change matching");
    }
    let mut x = Exchange::new();
    let mut out = Vec::with_capacity(64);
    let mut ex_lat = Vec::with_capacity(ex_cmds.len());
    for c in &ex_cmds {
        out.clear();
        let a = Instant::now();
        x.apply(c, &mut out);
        ex_lat.push((a.elapsed().as_nanos() as u32).saturating_sub(overhead));
    }
    ex_lat.sort_unstable();

    println!("Breakaway matching engine benchmark");
    println!("  machine           {} ({} threads available; engine uses 1)", cpu_model(), std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1));
    println!("  commands          {} ({} limit, {} immediate-or-cancel, {} cancels)", n, kinds[0], kinds[1], kinds[2]);
    println!("  fills             {}", fills);
    println!("  resting at end    {} orders", reference.resting_orders());
    println!("  throughput        {:.2} million commands/sec (best of 3, {:.3} s)", n as f64 / best / 1e6, best);
    println!("  latency per command, timer overhead ({} ns) subtracted:", overhead);
    for (label, p) in [("p50", 0.5), ("p90", 0.9), ("p99", 0.99), ("p99.9", 0.999), ("p99.99", 0.9999)] {
        println!("    {:<7} {:>6} ns", label, pct(&lat, p));
    }
    println!("    max     {:>6} ns", lat[lat.len() - 1]);
    println!("  with accounts, risk checks and escrow (same flow, well-funded accounts):");
    println!("    throughput {:.2} million commands/sec", ex_cmds.len() as f64 / ex_best / 1e6);
    for (label, p) in [("p50", 0.5), ("p99", 0.99), ("p99.9", 0.999)] {
        println!("    {:<7} {:>6} ns", label, pct(&ex_lat, p));
    }
}
