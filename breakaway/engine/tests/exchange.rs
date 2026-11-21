//! Accounts, risk checks and settlement.

use breakaway_engine::exchange::*;
use breakaway_engine::*;

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

fn go(x: &mut Exchange, cmd: ExCommand) -> Vec<ExEvent> {
    let mut out = Vec::new();
    x.apply(&cmd, &mut out);
    x.check().unwrap_or_else(|e| panic!("after {cmd:?}: {e}"));
    out
}
fn dep(account: AccountId, cents: i64) -> ExCommand {
    ExCommand::Deposit { account, cents }
}
fn buy(account: AccountId, price: u8, qty: u32) -> ExCommand {
    ExCommand::Place { account, side: Side::Buy, price, qty, tif: TimeInForce::Gtc }
}
fn sell(account: AccountId, price: u8, qty: u32) -> ExCommand {
    ExCommand::Place { account, side: Side::Sell, price, qty, tif: TimeInForce::Gtc }
}
fn acct(x: &Exchange, id: AccountId) -> Account {
    *x.account(id).unwrap()
}

#[test]
fn minting_a_pair_and_settling() {
    let mut x = Exchange::new();
    go(&mut x, dep(1, 10_000)); // $100
    go(&mut x, dep(2, 10_000));
    go(&mut x, buy(1, 60, 10)); // reserves 600
    assert_eq!(acct(&x, 1).cash_reserved, 600);
    go(&mut x, sell(2, 60, 10)); // no YES held: mints 10 pairs, reserves (100-60)*10 = 400
    let (a, b) = (acct(&x, 1), acct(&x, 2));
    assert_eq!((a.cash, a.yes), (10_000 - 600, 10));
    assert_eq!((b.cash, b.no), (10_000 - 400, 10));
    assert_eq!(x.escrow(), 1_000);
    go(&mut x, ExCommand::Settle { home_won: true });
    assert_eq!(acct(&x, 1).cash, 10_400, "YES paid 100 each: +400 profit");
    assert_eq!(acct(&x, 2).cash, 9_600);
    assert_eq!(x.escrow(), 0);
}

#[test]
fn selling_shares_you_own_and_buying_back_against_no() {
    let mut x = Exchange::new();
    for a in 1..=3 {
        go(&mut x, dep(a, 10_000));
    }
    go(&mut x, buy(1, 50, 4));
    go(&mut x, sell(2, 50, 4)); // 1 has 4 YES, 2 has 4 NO, escrow 400
    // 1 sells its YES to 3: covered by shares, no cash reserved
    go(&mut x, sell(1, 70, 4));
    assert_eq!((acct(&x, 1).yes_reserved, acct(&x, 1).cash_reserved), (4, 0));
    go(&mut x, buy(3, 70, 4));
    assert_eq!(acct(&x, 1).cash, 10_000 - 200 + 280, "bought at 50, sold at 70");
    assert_eq!(acct(&x, 3).yes, 4);
    // 2 buys YES while holding NO: the pairs are redeemed for 100 each
    go(&mut x, sell(3, 75, 4));
    go(&mut x, buy(2, 80, 4)); // trades at 75 (maker price), covered by 4 NO
    let b = acct(&x, 2);
    assert_eq!((b.no, b.yes), (0, 0));
    assert_eq!(b.cash, 10_000 - 200 + 4 * (100 - 75), "paid 50 per NO, got 25 back per pair");
    assert_eq!(x.escrow(), 0, "every pair redeemed");
}

#[test]
fn price_improvement_is_refunded() {
    let mut x = Exchange::new();
    go(&mut x, dep(1, 1_000));
    go(&mut x, dep(2, 1_000));
    go(&mut x, sell(2, 40, 5)); // reserves 60 x 5
    go(&mut x, buy(1, 55, 5)); // reserves 55 x 5, fills at 40
    assert_eq!(acct(&x, 1).cash, 1_000 - 200);
    assert_eq!(acct(&x, 1).cash_reserved, 0);
}

#[test]
fn risk_checks() {
    let mut x = Exchange::new();
    let r = go(&mut x, buy(9, 50, 1));
    assert_eq!(r, vec![ExEvent::RiskRejected { account: 9, reason: RiskReject::UnknownAccount }]);
    go(&mut x, dep(1, 1_000));
    assert!(matches!(go(&mut x, buy(1, 50, 21))[0], ExEvent::RiskRejected { reason: RiskReject::InsufficientFunds, .. }));
    go(&mut x, buy(1, 50, 20)); // exactly affordable
    assert!(matches!(go(&mut x, buy(1, 1, 1))[0], ExEvent::RiskRejected { reason: RiskReject::InsufficientFunds, .. }), "all cash is reserved");
    // cancelling frees it again
    go(&mut x, ExCommand::Cancel { account: 1, order_id: 1 });
    assert_eq!(acct(&x, 1), Account { cash: 1_000, ..Default::default() });
    go(&mut x, ExCommand::Settle { home_won: false });
    assert!(matches!(go(&mut x, buy(1, 50, 1))[0], ExEvent::RiskRejected { reason: RiskReject::MarketSettled, .. }));
}

#[test]
fn settlement_cancels_open_orders_first() {
    let mut x = Exchange::new();
    go(&mut x, dep(1, 5_000));
    go(&mut x, dep(2, 5_000));
    go(&mut x, buy(1, 30, 10));
    go(&mut x, sell(2, 90, 10));
    go(&mut x, ExCommand::Settle { home_won: true });
    assert_eq!(x.engine().resting_orders(), 0);
    assert_eq!((acct(&x, 1).cash, acct(&x, 2).cash), (5_000, 5_000));
}

/// Random order flow across many accounts and markets. Every invariant is
/// checked after every command, and each market is settled at the end: the
/// total money in the system must equal the total deposited.
#[test]
fn random_flow_keeps_the_books_balanced() {
    let markets = std::env::var("BREAKAWAY_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(30u64);
    let mut total = 0u64;
    let (mut fills, mut no_money, mut mints, mut redeems) = (0u64, 0u64, 0i64, 0i64);
    for seed in 1..=markets {
        let mut rng = Rng(0xD1B54A32D192ED03 ^ seed.wrapping_mul(0x9E3779B97F4A7C15));
        let mut x = Exchange::new();
        let mut log = Vec::new();
        let mut events = Vec::new();
        let n_accounts = 2 + rng.below(8) as u32;
        for a in 0..n_accounts {
            let c = ExCommand::Deposit { account: a, cents: 500 + rng.below(20_000) as i64 };
            x.apply(&c, &mut events);
            log.push(c);
        }
        let mut fair = 50i64;
        for _ in 0..20_000 {
            fair = (fair + rng.below(5) as i64 - 2).clamp(3, 97);
            let account = rng.below(n_accounts as u64 + 1) as u32; // sometimes unknown
            let cmd = match rng.below(100) {
                0..=27 => {
                    let resting = x.engine().snapshot();
                    if resting.is_empty() {
                        continue;
                    }
                    let (_, id, owner, _, _) = resting[rng.below(resting.len() as u64) as usize];
                    ExCommand::Cancel { account: if rng.below(15) == 0 { account } else { owner }, order_id: id }
                }
                28 => ExCommand::Deposit { account, cents: rng.below(3_000) as i64 },
                _ => {
                    let side = if rng.below(2) == 0 { Side::Buy } else { Side::Sell };
                    let price = (fair + rng.below(11) as i64 - 5).clamp(1, 99) as u8;
                    let tif = if rng.below(4) == 0 { TimeInForce::Ioc } else { TimeInForce::Gtc };
                    ExCommand::Place { account, side, price, qty: 1 + rng.below(40) as u32, tif }
                }
            };
            let before = (events.len(), x.escrow());
            x.apply(&cmd, &mut events);
            log.push(cmd);
            x.check().unwrap_or_else(|e| panic!("seed {seed} after {cmd:?}: {e}"));
            for ev in &events[before.0..] {
                match ev {
                    ExEvent::Book(Event::Fill { .. }) => fills += 1,
                    ExEvent::RiskRejected { reason: RiskReject::InsufficientFunds, .. } => no_money += 1,
                    _ => {}
                }
            }
            let d = x.escrow() - before.1;
            if d > 0 { mints += d / PAYOUT } else { redeems -= d / PAYOUT }
            total += 1;
        }
        let settle = ExCommand::Settle { home_won: rng.below(2) == 0 };
        x.apply(&settle, &mut events);
        log.push(settle);
        x.check().unwrap();
        assert_eq!(x.escrow(), 0);
        let cash: i64 = x.accounts().iter().map(|a| a.total_cash()).sum();
        assert_eq!(cash, x.deposits(), "seed {seed}: money created or destroyed");
        assert!(x.accounts().iter().all(|a| a.cash_reserved == 0 && a.yes_shares() == 0 && a.no_shares() == 0));

        // the whole market, including settlement, is reproducible from its log
        let mut y = Exchange::new();
        let mut events2 = Vec::new();
        for c in &log {
            y.apply(c, &mut events2);
        }
        assert_eq!(events, events2);
        assert_eq!(x.accounts(), y.accounts());
    }
    eprintln!("checked {total} commands across {markets} settled markets: {fills} fills, escrow grew by {mints} pairs and shrank by {redeems}, {no_money} orders refused for lack of funds");
    assert!(fills > total / 10 && mints > 0 && redeems > 0, "the random flow must exercise trading, minting and redeeming");
}
