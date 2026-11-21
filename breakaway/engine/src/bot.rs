//! The market-making bot, shared by the offline simulator and the live server.
//!
//! It sees the fair price late (the caller decides how late), quotes two levels
//! a side, widens its spread with recent volatility, shades its quotes against
//! its inventory, and pulls its quotes once the game is all but decided.

use crate::exchange::{ExCommand, Exchange};
use crate::{AccountId, OrderId, Side, TimeInForce};

pub struct MarketMaker {
    pub account: AccountId,
    orders: Vec<OrderId>,
    last_quote: (u8, u8, i64),
}

impl MarketMaker {
    pub fn new(account: AccountId) -> Self {
        MarketMaker { account, orders: Vec::new(), last_quote: (0, 0, 0) }
    }

    /// Commands to requote, given the fair price it currently sees (cents) and
    /// the one it saw about 20 seconds earlier. Empty if nothing changed.
    /// Call `track` with each resulting order that rests in the book.
    pub fn plan(&mut self, x: &Exchange, seen: f64, past: f64) -> Vec<ExCommand> {
        let vol = (seen - past).abs();
        let a = match x.account(self.account) {
            Some(a) => *a,
            None => return Vec::new(),
        };
        let inventory = a.yes_shares() - a.no_shares();
        let half = (1.5 + vol * 0.6).min(12.0);
        let skew = (inventory as f64 / 40.0).clamp(-6.0, 6.0);
        let bid = (seen - half - skew).floor().clamp(1.0, 98.0) as u8;
        let ask = ((seen + half - skew).ceil().clamp(2.0, 99.0) as u8).max(bid + 1);
        if (bid, ask, inventory) == self.last_quote {
            return Vec::new();
        }
        self.last_quote = (bid, ask, inventory);
        let mut cmds: Vec<ExCommand> = self.orders.drain(..).map(|order_id| ExCommand::Cancel { account: self.account, order_id }).collect();
        if seen > 1.0 && seen < 99.0 {
            for (price, qty, side) in [
                (bid, 40, Side::Buy),
                (bid.saturating_sub(3).max(1), 80, Side::Buy),
                (ask, 40, Side::Sell),
                ((ask + 3).min(99), 80, Side::Sell),
            ] {
                cmds.push(ExCommand::Place { account: self.account, side, price, qty, tif: TimeInForce::Gtc });
            }
        }
        cmds
    }

    pub fn track(&mut self, order_id: OrderId) {
        self.orders.push(order_id);
    }

    pub fn open_orders(&self) -> &[OrderId] {
        &self.orders
    }
}
