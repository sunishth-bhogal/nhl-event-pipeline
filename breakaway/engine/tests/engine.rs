//! Correctness tests for the matching engine.
//!
//! 1. Hand-written cases for each rule.
//! 2. Model-based testing: a deliberately simple reference book (a flat list,
//!    scanned in full for every match) receives the same random commands as
//!    the real engine, and every event must be identical. Invariants (no
//!    crossed book, quantities add up, conservation of contracts) are checked
//!    after every single command.
//! 3. Replay: rebuilding a book from its command log gives the same book.

use breakaway_engine::*;

// ------------------------------------------------------------------ helpers
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn run(e: &mut Engine, cmd: Command) -> Vec<Event> {
    let mut out = Vec::new();
    e.apply(&cmd, &mut out);
    out
}

fn buy(account: AccountId, price: u8, qty: u32) -> Command {
    Command::Place { account, side: Side::Buy, price, qty, tif: TimeInForce::Gtc }
}
fn sell(account: AccountId, price: u8, qty: u32) -> Command {
    Command::Place { account, side: Side::Sell, price, qty, tif: TimeInForce::Gtc }
}

// ------------------------------------------------------------------ reference
/// The obviously-correct version: a flat list of resting orders.
#[derive(Default)]
struct Reference {
    orders: Vec<(OrderId, AccountId, Side, u8, u32)>, // in arrival order
    next_id: OrderId,
}

impl Reference {
    fn apply(&mut self, cmd: &Command) -> Vec<Event> {
        if self.next_id == 0 {
            self.next_id = 1;
        }
        let mut out = Vec::new();
        match *cmd {
            Command::Place { account, side, price, qty, tif } => {
                if !(1..=99).contains(&price) {
                    out.push(Event::Rejected { reason: Reject::BadPrice });
                    return out;
                }
                if qty == 0 || qty > MAX_QTY {
                    out.push(Event::Rejected { reason: Reject::BadQty });
                    return out;
                }
                let id = self.next_id;
                self.next_id += 1;
                out.push(Event::Accepted { order_id: id, account, side, price, qty });
                let mut rem = qty;
                while rem > 0 {
                    // best crossing maker: best price, then earliest (lowest index)
                    let mut best: Option<usize> = None;
                    for (i, o) in self.orders.iter().enumerate() {
                        if o.2 == side {
                            continue;
                        }
                        let crosses = match side {
                            Side::Buy => o.3 <= price,
                            Side::Sell => o.3 >= price,
                        };
                        if !crosses {
                            continue;
                        }
                        let better = match best {
                            None => true,
                            Some(b) => match side {
                                Side::Buy => o.3 < self.orders[b].3,
                                Side::Sell => o.3 > self.orders[b].3,
                            },
                        };
                        if better {
                            best = Some(i);
                        }
                    }
                    let Some(i) = best else { break };
                    let m = self.orders[i];
                    if m.1 == account {
                        self.orders.remove(i);
                        out.push(Event::Cancelled { order_id: m.0, qty: m.4, reason: CancelReason::SelfTrade });
                        continue;
                    }
                    let q = rem.min(m.4);
                    rem -= q;
                    out.push(Event::Fill {
                        taker: id,
                        maker: m.0,
                        taker_account: account,
                        maker_account: m.1,
                        taker_side: side,
                        price: m.3,
                        qty: q,
                    });
                    if q == m.4 {
                        self.orders.remove(i);
                    } else {
                        self.orders[i].4 -= q;
                    }
                }
                if rem > 0 {
                    match tif {
                        TimeInForce::Gtc => {
                            self.orders.push((id, account, side, price, rem));
                            out.push(Event::Rested { order_id: id, qty: rem });
                        }
                        TimeInForce::Ioc => {
                            out.push(Event::Cancelled { order_id: id, qty: rem, reason: CancelReason::IocRemainder })
                        }
                    }
                }
            }
            Command::Cancel { account, order_id } => match self.orders.iter().position(|o| o.0 == order_id) {
                None => out.push(Event::Rejected { reason: Reject::UnknownOrder }),
                Some(i) if self.orders[i].1 != account => out.push(Event::Rejected { reason: Reject::NotOwner }),
                Some(i) => {
                    let o = self.orders.remove(i);
                    out.push(Event::Cancelled { order_id, qty: o.4, reason: CancelReason::Requested });
                }
            },
        }
        out
    }

    fn level_qty(&self, side: Side, price: u8) -> u64 {
        self.orders.iter().filter(|o| o.2 == side && o.3 == price).map(|o| o.4 as u64).sum()
    }
}

// ------------------------------------------------------------------ rule tests
#[test]
fn price_then_time_priority() {
    let mut e = Engine::new();
    run(&mut e, sell(1, 55, 10)); // id 1
    run(&mut e, sell(2, 54, 10)); // id 2: better price
    run(&mut e, sell(3, 54, 10)); // id 3: same price, later
    let ev = run(&mut e, buy(9, 60, 25));
    let fills: Vec<_> = ev
        .iter()
        .filter_map(|x| match x {
            Event::Fill { maker, price, qty, .. } => Some((*maker, *price, *qty)),
            _ => None,
        })
        .collect();
    assert_eq!(fills, vec![(2, 54, 10), (3, 54, 10), (1, 55, 5)]);
    assert_eq!(e.order(1), Some((Side::Sell, 55, 5, 1)));
    assert_eq!(e.best_bid(), None, "buy was fully filled, nothing rests");
}

#[test]
fn trades_at_maker_price_and_rests_remainder() {
    let mut e = Engine::new();
    run(&mut e, buy(1, 40, 5));
    let ev = run(&mut e, sell(2, 30, 8));
    assert!(ev.contains(&Event::Fill { taker: 2, maker: 1, taker_account: 2, maker_account: 1, taker_side: Side::Sell, price: 40, qty: 5 }));
    assert!(ev.contains(&Event::Rested { order_id: 2, qty: 3 }));
    assert_eq!((e.best_bid(), e.best_ask()), (None, Some(30)));
}

#[test]
fn ioc_never_rests() {
    let mut e = Engine::new();
    run(&mut e, sell(1, 50, 3));
    let ev = run(&mut e, Command::Place { account: 2, side: Side::Buy, price: 50, qty: 10, tif: TimeInForce::Ioc });
    assert_eq!(*ev.last().unwrap(), Event::Cancelled { order_id: 2, qty: 7, reason: CancelReason::IocRemainder });
    assert_eq!(e.resting_orders(), 0);
}

#[test]
fn self_trade_cancels_resting_order() {
    let mut e = Engine::new();
    run(&mut e, sell(7, 50, 4)); // id 1, same account as the taker
    run(&mut e, sell(8, 51, 4)); // id 2
    let ev = run(&mut e, buy(7, 51, 4));
    assert!(ev.contains(&Event::Cancelled { order_id: 1, qty: 4, reason: CancelReason::SelfTrade }));
    assert!(ev.iter().any(|x| matches!(x, Event::Fill { maker: 2, qty: 4, .. })));
}

#[test]
fn cancel_rules() {
    let mut e = Engine::new();
    run(&mut e, buy(1, 20, 5));
    assert_eq!(run(&mut e, Command::Cancel { account: 2, order_id: 1 }), vec![Event::Rejected { reason: Reject::NotOwner }]);
    assert_eq!(
        run(&mut e, Command::Cancel { account: 1, order_id: 1 }),
        vec![Event::Cancelled { order_id: 1, qty: 5, reason: CancelReason::Requested }]
    );
    assert_eq!(run(&mut e, Command::Cancel { account: 1, order_id: 1 }), vec![Event::Rejected { reason: Reject::UnknownOrder }]);
    assert_eq!(run(&mut e, Command::Cancel { account: 1, order_id: 999 }), vec![Event::Rejected { reason: Reject::UnknownOrder }]);
    assert_eq!(e.best_bid(), None);
}

#[test]
fn rejects_bad_input_without_using_an_id() {
    let mut e = Engine::new();
    assert_eq!(run(&mut e, buy(1, 0, 5)), vec![Event::Rejected { reason: Reject::BadPrice }]);
    assert_eq!(run(&mut e, buy(1, 100, 5)), vec![Event::Rejected { reason: Reject::BadPrice }]);
    assert_eq!(run(&mut e, buy(1, 50, 0)), vec![Event::Rejected { reason: Reject::BadQty }]);
    assert_eq!(run(&mut e, buy(1, 50, MAX_QTY + 1)), vec![Event::Rejected { reason: Reject::BadQty }]);
    assert!(matches!(run(&mut e, buy(1, 50, 1))[0], Event::Accepted { order_id: 1, .. }));
}

#[test]
fn extreme_prices_and_mask_edges() {
    let mut e = Engine::new();
    run(&mut e, buy(1, 1, 1));
    run(&mut e, sell(2, 99, 1));
    assert_eq!((e.best_bid(), e.best_ask()), (Some(1), Some(99)));
    run(&mut e, buy(3, 99, 2)); // takes the 99 ask, rests 1 at 99
    assert_eq!((e.best_bid(), e.best_ask()), (Some(99), None));
}

// ------------------------------------------------------------------ model-based
fn random_command(rng: &mut Rng, live: &[(OrderId, AccountId)], mid: &mut i64) -> Command {
    // a drifting "fair price", like a win probability moving during a game
    *mid = (*mid + rng.below(5) as i64 - 2).clamp(5, 95);
    let account = rng.below(6) as AccountId;
    let r = rng.below(100);
    if r < 30 && !live.is_empty() {
        let (id, owner) = live[rng.below(live.len() as u64) as usize];
        return match rng.below(20) {
            0 => Command::Cancel { account: owner + 1, order_id: id },          // wrong owner
            1 => Command::Cancel { account: owner, order_id: id + 1_000_000 },  // no such order
            _ => Command::Cancel { account: owner, order_id: id },
        };
    }
    if r < 32 {
        // invalid input
        let price = [0, 100, 50][rng.below(3) as usize];
        let qty = [5, 5, 0][rng.below(3) as usize];
        return Command::Place { account, side: Side::Buy, price, qty, tif: TimeInForce::Gtc };
    }
    let side = if rng.below(2) == 0 { Side::Buy } else { Side::Sell };
    let offset = rng.below(9) as i64 - 4;
    let price = (*mid + offset).clamp(1, 99) as u8;
    let size_cap = if rng.below(20) == 0 { 500 } else { 20 };
    let qty = 1 + rng.below(size_cap) as u32;
    let tif = if rng.below(5) == 0 { TimeInForce::Ioc } else { TimeInForce::Gtc };
    Command::Place { account, side, price, qty, tif }
}

fn check_invariants(e: &Engine, r: &Reference) {
    if let (Some(b), Some(a)) = (e.best_bid(), e.best_ask()) {
        assert!(b < a, "crossed book: bid {b} >= ask {a}");
    }
    for p in 1..=99u8 {
        for side in [Side::Buy, Side::Sell] {
            assert_eq!(e.level_qty(side, p), r.level_qty(side, p), "level {side:?} {p}");
        }
    }
    assert_eq!(e.resting_orders(), r.orders.len());
}

/// Every contract the taker asked for is accounted for: filled, rested or cancelled.
fn check_conservation(cmd: &Command, events: &[Event]) {
    if let Command::Place { qty, .. } = *cmd {
        if let Some(Event::Accepted { order_id, .. }) = events.first() {
            let mut seen: u64 = 0;
            for ev in events {
                match *ev {
                    Event::Fill { taker, qty, .. } if taker == *order_id => seen += qty as u64,
                    Event::Rested { order_id: o, qty } if o == *order_id => seen += qty as u64,
                    Event::Cancelled { order_id: o, qty, reason: CancelReason::IocRemainder } if o == *order_id => {
                        seen += qty as u64
                    }
                    _ => {}
                }
            }
            assert_eq!(seen, qty as u64, "order {order_id} quantities do not add up");
        }
    }
}

#[test]
fn matches_reference_on_random_order_flow() {
    let seeds = std::env::var("BREAKAWAY_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(40u64);
    let steps = 25_000;
    let mut total = 0u64;
    for seed in 1..=seeds {
        let mut rng = Rng(0x9E3779B97F4A7C15 ^ seed.wrapping_mul(0x100000001B3));
        let mut e = Engine::new();
        let mut r = Reference::default();
        let mut mid = 50i64;
        let mut out = Vec::new();
        for step in 0..steps {
            let live: Vec<(OrderId, AccountId)> = r.orders.iter().map(|o| (o.0, o.1)).collect();
            let cmd = random_command(&mut rng, &live, &mut mid);
            out.clear();
            e.apply(&cmd, &mut out);
            let expected = r.apply(&cmd);
            assert_eq!(out, expected, "seed {seed} step {step}: {cmd:?}");
            check_conservation(&cmd, &out);
            check_invariants(&e, &r);
            total += 1;
        }
    }
    eprintln!("checked {total} random commands against the reference book");
}

#[test]
fn replaying_the_log_rebuilds_the_same_book() {
    let mut rng = Rng(42);
    let mut e = Engine::new();
    let mut log = Vec::new();
    let mut events = Vec::new();
    let mut mid = 50i64;
    for _ in 0..200_000 {
        let live: Vec<(OrderId, AccountId)> = e.snapshot().iter().map(|o| (o.1, o.2)).collect();
        let cmd = random_command(&mut rng, &live, &mut mid);
        e.apply(&cmd, &mut events);
        log.push(cmd);
    }
    let mut replay = Engine::new();
    let mut replay_events = Vec::new();
    for cmd in &log {
        replay.apply(cmd, &mut replay_events);
    }
    assert_eq!(events, replay_events);
    assert_eq!(e.state_hash(), replay.state_hash());
    assert_eq!(e.snapshot(), replay.snapshot());
}
