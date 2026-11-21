//! Play-money exchange around the matching engine: accounts, pre-trade risk
//! checks, a fully collateralised escrow, and settlement.
//!
//! Money model
//! -----------
//! Every contract is one half of a pair: a YES share ("home wins") and a NO
//! share ("home doesn't win") together always pay exactly 100¢. All amounts
//! are integer cents.
//!
//! * Buying YES at p costs p. If the buyer already holds NO, the new YES and
//!   an old NO are redeemed as a pair for 100 from escrow instead.
//! * Selling YES at p either hands over a YES share the seller holds, or
//!   mints a new pair: the seller puts 100 into escrow, keeps the NO share and
//!   receives p from the buyer (a net cost of 100 - p).
//! * Escrow therefore always holds exactly 100 per outstanding pair, so the
//!   exchange never has counterparty risk.
//!
//! Risk check
//! ----------
//! An order is accepted only if it can be paid for in the worst case, i.e. if
//! it fully fills at its limit. Shares the account already holds are used as
//! cover first (selling YES you own, or buying YES against NO you own); the
//! rest is reserved in cash: p per contract for a buy, 100 - p for a sell.
//! Fills at a better price than the limit refund the difference immediately.
//!
//! Invariants (checked by `Exchange::check` and asserted in the tests)
//! ------------------------------------------------------------------
//! * Conservation: sum of all cash (free + reserved) + escrow == deposits.
//! * Escrow == 100 x YES shares outstanding == 100 x NO shares outstanding.
//! * Nothing is ever negative.
//! * Reserved cash and shares equal what the open orders need, and every open
//!   order's remaining quantity equals what is resting in the book.

use crate::{AccountId, Command, Engine, Event, OrderId, Side, TimeInForce, MAX_PRICE, MAX_QTY, MIN_PRICE};

pub const PAYOUT: i64 = 100;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Account {
    pub cash: i64,
    pub cash_reserved: i64,
    pub yes: i64,
    pub yes_reserved: i64,
    pub no: i64,
    pub no_reserved: i64,
}

impl Account {
    pub fn total_cash(&self) -> i64 {
        self.cash + self.cash_reserved
    }
    pub fn yes_shares(&self) -> i64 {
        self.yes + self.yes_reserved
    }
    pub fn no_shares(&self) -> i64 {
        self.no + self.no_reserved
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExCommand {
    Deposit { account: AccountId, cents: i64 },
    Place { account: AccountId, side: Side, price: u8, qty: u32, tif: TimeInForce },
    Cancel { account: AccountId, order_id: OrderId },
    /// The game is over: cancel every open order and pay out the escrow.
    Settle { home_won: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiskReject {
    UnknownAccount,
    InsufficientFunds,
    MarketSettled,
    BadDeposit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExEvent {
    Book(Event),
    RiskRejected { account: AccountId, reason: RiskReject },
    Deposited { account: AccountId, cents: i64 },
    Settled { home_won: bool, paid_out: i64 },
}

/// What an open order still needs reserved.
#[derive(Clone, Copy, Debug)]
struct Open {
    account: AccountId,
    side: Side,
    limit: u8,
    /// contracts covered by shares the account holds (NO for buys, YES for sells)
    covered: u32,
    /// contracts backed by reserved cash
    uncovered: u32,
}

pub struct Exchange {
    engine: Engine,
    accounts: Vec<Account>,
    open: Vec<Option<Open>>, // by order id
    escrow: i64,
    deposits: i64,
    settled: Option<bool>,
    scratch: Vec<Event>,
}

impl Default for Exchange {
    fn default() -> Self {
        Self::new()
    }
}

impl Exchange {
    pub fn new() -> Self {
        Exchange {
            engine: Engine::with_capacity(4096),
            accounts: Vec::new(),
            open: vec![None],
            escrow: 0,
            deposits: 0,
            settled: None,
            scratch: Vec::with_capacity(64),
        }
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }
    pub fn account(&self, id: AccountId) -> Option<&Account> {
        self.accounts.get(id as usize)
    }
    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }
    pub fn escrow(&self) -> i64 {
        self.escrow
    }
    pub fn deposits(&self) -> i64 {
        self.deposits
    }
    pub fn settled(&self) -> Option<bool> {
        self.settled
    }

    pub fn apply(&mut self, cmd: &ExCommand, out: &mut Vec<ExEvent>) {
        match *cmd {
            ExCommand::Deposit { account, cents } => self.deposit(account, cents, out),
            ExCommand::Place { account, side, price, qty, tif } => self.place(account, side, price, qty, tif, out),
            ExCommand::Cancel { account, order_id } => self.cancel(account, order_id, out),
            ExCommand::Settle { home_won } => self.settle(home_won, out),
        }
    }

    // ------------------------------------------------------------ commands
    fn deposit(&mut self, account: AccountId, cents: i64, out: &mut Vec<ExEvent>) {
        if cents <= 0 || self.settled.is_some() {
            let reason = if cents <= 0 { RiskReject::BadDeposit } else { RiskReject::MarketSettled };
            out.push(ExEvent::RiskRejected { account, reason });
            return;
        }
        let i = account as usize;
        if self.accounts.len() <= i {
            self.accounts.resize(i + 1, Account::default());
        }
        self.accounts[i].cash += cents;
        self.deposits += cents;
        out.push(ExEvent::Deposited { account, cents });
    }

    fn place(&mut self, account: AccountId, side: Side, price: u8, qty: u32, tif: TimeInForce, out: &mut Vec<ExEvent>) {
        if self.settled.is_some() {
            out.push(ExEvent::RiskRejected { account, reason: RiskReject::MarketSettled });
            return;
        }
        let Some(acct) = self.accounts.get(account as usize).copied() else {
            out.push(ExEvent::RiskRejected { account, reason: RiskReject::UnknownAccount });
            return;
        };
        let valid = (MIN_PRICE..=MAX_PRICE).contains(&price) && qty > 0 && qty <= MAX_QTY;
        let mut reservation = None;
        if valid {
            // cover with shares the account holds, reserve cash for the rest
            let (covered, per_contract) = match side {
                Side::Buy => ((qty as i64).min(acct.no) as u32, price as i64),
                Side::Sell => ((qty as i64).min(acct.yes) as u32, PAYOUT - price as i64),
            };
            let need = (qty - covered) as i64 * per_contract;
            if need > acct.cash {
                out.push(ExEvent::RiskRejected { account, reason: RiskReject::InsufficientFunds });
                return;
            }
            let a = &mut self.accounts[account as usize];
            match side {
                Side::Buy => {
                    a.no -= covered as i64;
                    a.no_reserved += covered as i64;
                }
                Side::Sell => {
                    a.yes -= covered as i64;
                    a.yes_reserved += covered as i64;
                }
            }
            a.cash -= need;
            a.cash_reserved += need;
            reservation = Some(Open { account, side, limit: price, covered, uncovered: qty - covered });
        }
        // invalid input still goes to the engine so it is rejected the same way everywhere
        let id = self.engine.next_order_id();
        if let Some(r) = reservation {
            if self.open.len() <= id as usize {
                self.open.resize(id as usize + 1, None);
            }
            self.open[id as usize] = Some(r);
        }
        self.run(Command::Place { account, side, price, qty, tif }, out);
    }

    fn cancel(&mut self, account: AccountId, order_id: OrderId, out: &mut Vec<ExEvent>) {
        self.run(Command::Cancel { account, order_id }, out);
    }

    fn settle(&mut self, home_won: bool, out: &mut Vec<ExEvent>) {
        if self.settled.is_some() {
            out.push(ExEvent::RiskRejected { account: 0, reason: RiskReject::MarketSettled });
            return;
        }
        // 1. cancel everything still resting, releasing its reservations
        for (_, id, owner, _, _) in self.engine.snapshot() {
            self.run(Command::Cancel { account: owner, order_id: id }, out);
        }
        // 2. pay each winning share 100 from escrow; losing shares expire
        let mut paid = 0;
        for a in self.accounts.iter_mut() {
            let winners = if home_won { a.yes_shares() } else { a.no_shares() };
            a.cash += winners * PAYOUT;
            paid += winners * PAYOUT;
            a.yes = 0;
            a.no = 0;
            a.yes_reserved = 0;
            a.no_reserved = 0;
        }
        self.escrow -= paid;
        self.settled = Some(home_won);
        out.push(ExEvent::Settled { home_won, paid_out: paid });
    }

    // ------------------------------------------------------------ book events -> ledger
    fn run(&mut self, cmd: Command, out: &mut Vec<ExEvent>) {
        let mut evs = std::mem::take(&mut self.scratch);
        evs.clear();
        self.engine.apply(&cmd, &mut evs);
        for ev in &evs {
            match *ev {
                Event::Fill { taker, maker, taker_side, price, qty, .. } => {
                    let (buy, sell) = match taker_side {
                        Side::Buy => (taker, maker),
                        Side::Sell => (maker, taker),
                    };
                    self.fill(buy, sell, price, qty);
                }
                Event::Cancelled { order_id, qty, .. } => self.release(order_id, qty),
                _ => {}
            }
            out.push(ExEvent::Book(*ev));
        }
        // drop bookkeeping for orders that are completely done
        for ev in &evs {
            if let Event::Accepted { order_id, .. } = *ev {
                if self.engine.order(order_id).is_none() {
                    if let Some(Some(o)) = self.open.get(order_id as usize) {
                        debug_assert_eq!(o.covered + o.uncovered, 0);
                    }
                    if let Some(slot) = self.open.get_mut(order_id as usize) {
                        *slot = None;
                    }
                }
            }
        }
        self.scratch = evs;
    }

    fn fill(&mut self, buy: OrderId, sell: OrderId, price: u8, qty: u32) {
        let t = price as i64;
        // buyer: covered contracts redeem a pair (NO + new YES) for 100
        let b = self.open[buy as usize].as_mut().expect("buy order");
        let bc = qty.min(b.covered);
        let bu = qty - bc;
        b.covered -= bc;
        b.uncovered -= bu;
        let (b_acct, b_limit) = (b.account, b.limit as i64);
        // seller: covered contracts hand over YES; the rest mint a new pair
        let s = self.open[sell as usize].as_mut().expect("sell order");
        let sc = qty.min(s.covered);
        let su = qty - sc;
        s.covered -= sc;
        s.uncovered -= su;
        let (s_acct, s_limit) = (s.account, s.limit as i64);

        {
            let a = &mut self.accounts[b_acct as usize];
            a.no_reserved -= bc as i64;
            a.cash += (PAYOUT - t) * bc as i64;
            a.cash_reserved -= b_limit * bu as i64;
            a.cash += (b_limit - t) * bu as i64; // price improvement back
            a.yes += bu as i64;
        }
        {
            let a = &mut self.accounts[s_acct as usize];
            a.yes_reserved -= sc as i64;
            a.cash += t * sc as i64;
            a.cash_reserved -= (PAYOUT - s_limit) * su as i64;
            a.cash += (t - s_limit) * su as i64; // price improvement back
            a.no += su as i64;
        }
        self.escrow += PAYOUT * (su as i64 - bc as i64);
    }

    fn release(&mut self, order_id: OrderId, qty: u32) {
        let Some(o) = self.open.get_mut(order_id as usize).and_then(|x| x.take()) else { return };
        debug_assert_eq!(o.covered + o.uncovered, qty, "cancel must release the whole remainder");
        let a = &mut self.accounts[o.account as usize];
        match o.side {
            Side::Buy => {
                a.no_reserved -= o.covered as i64;
                a.no += o.covered as i64;
                a.cash_reserved -= o.uncovered as i64 * o.limit as i64;
                a.cash += o.uncovered as i64 * o.limit as i64;
            }
            Side::Sell => {
                a.yes_reserved -= o.covered as i64;
                a.yes += o.covered as i64;
                a.cash_reserved -= o.uncovered as i64 * (PAYOUT - o.limit as i64);
                a.cash += o.uncovered as i64 * (PAYOUT - o.limit as i64);
            }
        }
    }

    // ------------------------------------------------------------ invariants
    /// Verifies every accounting invariant; returns a description of the first violation.
    pub fn check(&self) -> Result<(), String> {
        let mut cash = 0i64;
        let mut yes = 0i64;
        let mut no = 0i64;
        let mut need = vec![(0i64, 0i64, 0i64); self.accounts.len()]; // cash, yes, no reserved
        for (i, a) in self.accounts.iter().enumerate() {
            for (name, v) in [("cash", a.cash), ("cash_reserved", a.cash_reserved), ("yes", a.yes), ("yes_reserved", a.yes_reserved), ("no", a.no), ("no_reserved", a.no_reserved)] {
                if v < 0 {
                    return Err(format!("account {i}: {name} is negative ({v})"));
                }
            }
            cash += a.total_cash();
            yes += a.yes_shares();
            no += a.no_shares();
        }
        if cash + self.escrow != self.deposits {
            return Err(format!("money not conserved: cash {cash} + escrow {} != deposits {}", self.escrow, self.deposits));
        }
        if self.escrow != PAYOUT * yes || self.escrow != PAYOUT * no {
            return Err(format!("escrow {} != 100 x pairs (yes {yes}, no {no})", self.escrow));
        }
        for (id, o) in self.open.iter().enumerate() {
            let Some(o) = o else { continue };
            let resting = self.engine.order(id as OrderId).map(|x| x.2).unwrap_or(0);
            if resting != o.covered + o.uncovered {
                return Err(format!("order {id}: book has {resting}, ledger reserves for {}", o.covered + o.uncovered));
            }
            let n = &mut need[o.account as usize];
            match o.side {
                Side::Buy => {
                    n.0 += o.uncovered as i64 * o.limit as i64;
                    n.2 += o.covered as i64;
                }
                Side::Sell => {
                    n.0 += o.uncovered as i64 * (PAYOUT - o.limit as i64);
                    n.1 += o.covered as i64;
                }
            }
        }
        for (i, (a, n)) in self.accounts.iter().zip(&need).enumerate() {
            if (a.cash_reserved, a.yes_reserved, a.no_reserved) != *n {
                return Err(format!("account {i}: reserved {:?} but open orders need {:?}", (a.cash_reserved, a.yes_reserved, a.no_reserved), n));
            }
        }
        for (_, id, ..) in self.engine.snapshot() {
            if self.open.get(id as usize).map_or(true, |o| o.is_none()) {
                return Err(format!("order {id} rests in the book without a reservation"));
            }
        }
        Ok(())
    }
}
