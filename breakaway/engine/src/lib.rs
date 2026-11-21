//! Breakaway matching engine.
//!
//! One order book per game for a binary contract: "the home team wins". It
//! trades between 1 and 99 cents and settles at 100 if the home team wins,
//! 0 otherwise, so a price is a win probability.
//!
//! Rules
//! * Price-time priority: best price first, then earliest order at that price.
//! * Trades happen at the resting (maker) order's price.
//! * Good-'til-cancelled orders rest; immediate-or-cancel orders never rest.
//! * Self-trade prevention: an order never fills against its own account;
//!   the resting order is cancelled instead.
//! * Deterministic: the same commands always produce the same events and the
//!   same book, so the book can be rebuilt by replaying its command log.
//!
//! Structure
//! * 99 price levels per side, each a FIFO queue threaded through a slab of
//!   order nodes (intrusive doubly linked list), so add, cancel and fill are
//!   O(1) with no allocation after warm-up.
//! * A 128-bit mask per side marks non-empty levels; the best bid is the
//!   highest set bit and the best ask the lowest, so finding the top of book
//!   is a single instruction.

pub type OrderId = u64;
pub type AccountId = u32;

pub const MIN_PRICE: u8 = 1;
pub const MAX_PRICE: u8 = 99;
pub const MAX_QTY: u32 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimeInForce {
    /// Good 'til cancelled: any remainder rests in the book.
    Gtc,
    /// Immediate or cancel: any remainder is cancelled.
    Ioc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Place { account: AccountId, side: Side, price: u8, qty: u32, tif: TimeInForce },
    Cancel { account: AccountId, order_id: OrderId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    BadPrice,
    BadQty,
    UnknownOrder,
    NotOwner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelReason {
    Requested,
    IocRemainder,
    SelfTrade,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Accepted { order_id: OrderId, account: AccountId, side: Side, price: u8, qty: u32 },
    Fill {
        taker: OrderId,
        maker: OrderId,
        taker_account: AccountId,
        maker_account: AccountId,
        taker_side: Side,
        price: u8,
        qty: u32,
    },
    Rested { order_id: OrderId, qty: u32 },
    Cancelled { order_id: OrderId, qty: u32, reason: CancelReason },
    Rejected { reason: Reject },
}

const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
struct Node {
    id: OrderId,
    account: AccountId,
    qty: u32,
    price: u8,
    side: Side,
    prev: u32,
    next: u32,
}

#[derive(Clone, Copy, Debug)]
struct Level {
    head: u32,
    tail: u32,
    total: u64,
    count: u32,
}

const EMPTY_LEVEL: Level = Level { head: NONE, tail: NONE, total: 0, count: 0 };

pub struct Engine {
    nodes: Vec<Node>,
    free: Vec<u32>,
    /// order id -> slab slot (NONE once the order is gone). Ids are dense.
    slot_of: Vec<u32>,
    bids: [Level; 100],
    asks: [Level; 100],
    bid_mask: u128,
    ask_mask: u128,
    next_id: OrderId,
    commands: u64,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self::with_capacity(1024)
    }

    pub fn with_capacity(resting_orders: usize) -> Self {
        Engine {
            nodes: Vec::with_capacity(resting_orders),
            free: Vec::with_capacity(resting_orders),
            slot_of: vec![NONE], // ids start at 1
            bids: [EMPTY_LEVEL; 100],
            asks: [EMPTY_LEVEL; 100],
            bid_mask: 0,
            ask_mask: 0,
            next_id: 1,
            commands: 0,
        }
    }

    // ------------------------------------------------------------ queries
    pub fn best_bid(&self) -> Option<u8> {
        if self.bid_mask == 0 {
            None
        } else {
            Some((127 - self.bid_mask.leading_zeros()) as u8)
        }
    }

    pub fn best_ask(&self) -> Option<u8> {
        if self.ask_mask == 0 {
            None
        } else {
            Some(self.ask_mask.trailing_zeros() as u8)
        }
    }

    /// Total resting quantity at a price on one side.
    pub fn level_qty(&self, side: Side, price: u8) -> u64 {
        self.levels(side)[price as usize].total
    }

    /// Up to `n` levels from the top of book: (price, quantity, order count).
    pub fn depth(&self, side: Side, n: usize) -> Vec<(u8, u64, u32)> {
        let mut out = Vec::with_capacity(n);
        let mut mask = match side {
            Side::Buy => self.bid_mask,
            Side::Sell => self.ask_mask,
        };
        while mask != 0 && out.len() < n {
            let p = match side {
                Side::Buy => 127 - mask.leading_zeros(),
                Side::Sell => mask.trailing_zeros(),
            } as usize;
            let l = &self.levels(side)[p];
            out.push((p as u8, l.total, l.count));
            mask &= !(1u128 << p);
        }
        out
    }

    /// A resting order: (side, price, remaining quantity, account).
    pub fn order(&self, id: OrderId) -> Option<(Side, u8, u32, AccountId)> {
        let slot = *self.slot_of.get(id as usize)?;
        if slot == NONE {
            return None;
        }
        let n = &self.nodes[slot as usize];
        Some((n.side, n.price, n.qty, n.account))
    }

    pub fn resting_orders(&self) -> usize {
        self.nodes.len() - self.free.len()
    }

    pub fn commands_applied(&self) -> u64 {
        self.commands
    }

    /// Every resting order in priority order, per side: (id, account, price, qty).
    /// Used for snapshots and to check that a replay rebuilt the same book.
    pub fn snapshot(&self) -> Vec<(Side, OrderId, AccountId, u8, u32)> {
        let mut out = Vec::with_capacity(self.resting_orders());
        for side in [Side::Buy, Side::Sell] {
            for (p, _, _) in self.depth(side, 100) {
                let mut s = self.levels(side)[p as usize].head;
                while s != NONE {
                    let n = &self.nodes[s as usize];
                    out.push((side, n.id, n.account, n.price, n.qty));
                    s = n.next;
                }
            }
        }
        out
    }

    /// FNV-1a over the snapshot plus the id counter: equal books, equal hash.
    pub fn state_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut eat = |v: u64| {
            for b in v.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        eat(self.next_id);
        for (side, id, acct, price, qty) in self.snapshot() {
            eat(side as u64);
            eat(id);
            eat(acct as u64);
            eat(price as u64);
            eat(qty as u64);
        }
        h
    }

    // ------------------------------------------------------------ commands
    /// Apply one command, appending its events to `out`.
    pub fn apply(&mut self, cmd: &Command, out: &mut Vec<Event>) {
        self.commands += 1;
        match *cmd {
            Command::Place { account, side, price, qty, tif } => self.place(account, side, price, qty, tif, out),
            Command::Cancel { account, order_id } => self.cancel(account, order_id, out),
        }
    }

    fn place(&mut self, account: AccountId, side: Side, price: u8, qty: u32, tif: TimeInForce, out: &mut Vec<Event>) {
        if !(MIN_PRICE..=MAX_PRICE).contains(&price) {
            out.push(Event::Rejected { reason: Reject::BadPrice });
            return;
        }
        if qty == 0 || qty > MAX_QTY {
            out.push(Event::Rejected { reason: Reject::BadQty });
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.slot_of.push(NONE);
        out.push(Event::Accepted { order_id: id, account, side, price, qty });

        let mut remaining = qty;
        while remaining > 0 {
            // best opposite price that crosses our limit
            let level_price = match side {
                Side::Buy => match self.best_ask() {
                    Some(a) if a <= price => a,
                    _ => break,
                },
                Side::Sell => match self.best_bid() {
                    Some(b) if b >= price => b,
                    _ => break,
                },
            };
            let maker_side = opposite(side);
            let slot = self.levels(maker_side)[level_price as usize].head;
            let maker = self.nodes[slot as usize];
            if maker.account == account {
                self.remove(slot);
                out.push(Event::Cancelled { order_id: maker.id, qty: maker.qty, reason: CancelReason::SelfTrade });
                continue;
            }
            let q = remaining.min(maker.qty);
            remaining -= q;
            out.push(Event::Fill {
                taker: id,
                maker: maker.id,
                taker_account: account,
                maker_account: maker.account,
                taker_side: side,
                price: level_price,
                qty: q,
            });
            if q == maker.qty {
                self.remove(slot);
            } else {
                self.nodes[slot as usize].qty -= q;
                self.levels_mut(maker_side)[level_price as usize].total -= q as u64;
            }
        }

        if remaining > 0 {
            match tif {
                TimeInForce::Gtc => {
                    self.insert(id, account, side, price, remaining);
                    out.push(Event::Rested { order_id: id, qty: remaining });
                }
                TimeInForce::Ioc => {
                    out.push(Event::Cancelled { order_id: id, qty: remaining, reason: CancelReason::IocRemainder });
                }
            }
        }
    }

    fn cancel(&mut self, account: AccountId, order_id: OrderId, out: &mut Vec<Event>) {
        let slot = match self.slot_of.get(order_id as usize) {
            Some(&s) if s != NONE => s,
            _ => {
                out.push(Event::Rejected { reason: Reject::UnknownOrder });
                return;
            }
        };
        let n = self.nodes[slot as usize];
        if n.account != account {
            out.push(Event::Rejected { reason: Reject::NotOwner });
            return;
        }
        self.remove(slot);
        out.push(Event::Cancelled { order_id, qty: n.qty, reason: CancelReason::Requested });
    }

    // ------------------------------------------------------------ internals
    #[inline]
    fn levels(&self, side: Side) -> &[Level; 100] {
        match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        }
    }

    #[inline]
    fn levels_mut(&mut self, side: Side) -> &mut [Level; 100] {
        match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        }
    }

    fn insert(&mut self, id: OrderId, account: AccountId, side: Side, price: u8, qty: u32) {
        let p = price as usize;
        let tail = self.levels(side)[p].tail;
        let node = Node { id, account, qty, price, side, prev: tail, next: NONE };
        let slot = match self.free.pop() {
            Some(s) => {
                self.nodes[s as usize] = node;
                s
            }
            None => {
                self.nodes.push(node);
                (self.nodes.len() - 1) as u32
            }
        };
        if tail != NONE {
            self.nodes[tail as usize].next = slot;
        }
        let level = &mut self.levels_mut(side)[p];
        if level.head == NONE {
            level.head = slot;
        }
        level.tail = slot;
        level.total += qty as u64;
        level.count += 1;
        match side {
            Side::Buy => self.bid_mask |= 1u128 << p,
            Side::Sell => self.ask_mask |= 1u128 << p,
        }
        self.slot_of[id as usize] = slot;
    }

    fn remove(&mut self, slot: u32) {
        let n = self.nodes[slot as usize];
        if n.prev != NONE {
            self.nodes[n.prev as usize].next = n.next;
        }
        if n.next != NONE {
            self.nodes[n.next as usize].prev = n.prev;
        }
        let p = n.price as usize;
        let level = &mut self.levels_mut(n.side)[p];
        if level.head == slot {
            level.head = n.next;
        }
        if level.tail == slot {
            level.tail = n.prev;
        }
        level.total -= n.qty as u64;
        level.count -= 1;
        if level.count == 0 {
            match n.side {
                Side::Buy => self.bid_mask &= !(1u128 << p),
                Side::Sell => self.ask_mask &= !(1u128 << p),
            }
        }
        self.slot_of[n.id as usize] = NONE;
        self.free.push(slot);
    }
}

#[inline]
pub fn opposite(s: Side) -> Side {
    match s {
        Side::Buy => Side::Sell,
        Side::Sell => Side::Buy,
    }
}
