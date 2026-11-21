//! Write-ahead journal for an exchange.
//!
//! Every command is appended to the journal and flushed to disk *before* the
//! client hears back, so anything acknowledged survives a crash. On restart the
//! journal is replayed into a fresh `Exchange`; matching is deterministic, so
//! the rebuilt state is identical (`tests/journal.rs` checks this, including a
//! crash in the middle of writing a line).
//!
//! Format: one command per line, plain text so a human can read it.
//!     D <account> <cents>                    deposit
//!     P <account> <B|S> <price> <qty> <G|I>  place (good-'til-cancelled / IOC)
//!     C <account> <order id>                 cancel
//!     S <1|0>                                settle (home won?)
//!     T <game second>                        clock marker, not an exchange command
//!     J <account> <token>                    a trader joined (token: 16 hex digits)
//! A line without its trailing newline was torn by a crash and is ignored.

use crate::exchange::{ExCommand, ExEvent, Exchange};
use crate::{Side, TimeInForce};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Cmd(ExCommand),
    Clock(u32),
    Join { account: u32, token: u64 },
}

pub fn encode(e: &Entry) -> String {
    match *e {
        Entry::Cmd(ExCommand::Deposit { account, cents }) => format!("D {account} {cents}"),
        Entry::Cmd(ExCommand::Place { account, side, price, qty, tif }) => format!(
            "P {account} {} {price} {qty} {}",
            if side == Side::Buy { 'B' } else { 'S' },
            if tif == TimeInForce::Gtc { 'G' } else { 'I' }
        ),
        Entry::Cmd(ExCommand::Cancel { account, order_id }) => format!("C {account} {order_id}"),
        Entry::Cmd(ExCommand::Settle { home_won }) => format!("S {}", home_won as u8),
        Entry::Clock(sec) => format!("T {sec}"),
        Entry::Join { account, token } => format!("J {account} {token:016x}"),
    }
}

pub fn decode(line: &str) -> Option<Entry> {
    let mut it = line.split(' ');
    let tag = it.next()?;
    let mut num = || it.next()?.parse::<i64>().ok();
    let e = match tag {
        "D" => Entry::Cmd(ExCommand::Deposit { account: num()? as u32, cents: num()? }),
        "C" => Entry::Cmd(ExCommand::Cancel { account: num()? as u32, order_id: num()? as u64 }),
        "S" => Entry::Cmd(ExCommand::Settle { home_won: num()? == 1 }),
        "T" => Entry::Clock(num()? as u32),
        "J" => {
            let account = it.next()?.parse().ok()?;
            let token = u64::from_str_radix(it.next()?, 16).ok()?;
            Entry::Join { account, token }
        }
        "P" => {
            let account = it.next()?.parse().ok()?;
            let side = match it.next()? { "B" => Side::Buy, "S" => Side::Sell, _ => return None };
            let price = it.next()?.parse().ok()?;
            let qty = it.next()?.parse().ok()?;
            let tif = match it.next()? { "G" => TimeInForce::Gtc, "I" => TimeInForce::Ioc, _ => return None };
            Entry::Cmd(ExCommand::Place { account, side, price, qty, tif })
        }
        _ => return None,
    };
    Some(e)
}

pub struct Journal {
    out: BufWriter<File>,
    pub entries: u64,
}

impl Journal {
    /// Opens (or creates) the journal at `path`, replays it, and returns the
    /// rebuilt exchange, the last clock marker, and a writer positioned at the end.
    pub fn open(path: &Path) -> std::io::Result<(Journal, Exchange, Option<u32>)> {
        let (j, x, clock, _) = Self::open_with_joins(path)?;
        Ok((j, x, clock))
    }

    /// Like `open`, also returning every (account, token) that joined.
    pub fn open_with_joins(path: &Path) -> std::io::Result<(Journal, Exchange, Option<u32>, Vec<(u32, u64)>)> {
        let mut joins = Vec::new();
        let mut x = Exchange::new();
        let mut clock = None;
        let mut entries = 0u64;
        let mut good_len = 0u64;
        if path.exists() {
            let mut raw = Vec::new();
            File::open(path)?.read_to_end(&mut raw)?;
            let mut out = Vec::<ExEvent>::new();
            for line in BufReader::new(&raw[..]).split(b'\n') {
                let line = line?;
                let end = good_len as usize + line.len();
                if end >= raw.len() {
                    break; // last line has no newline: torn write, ignore it
                }
                let text = std::str::from_utf8(&line).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                match decode(text) {
                    Some(Entry::Cmd(c)) => {
                        out.clear();
                        x.apply(&c, &mut out);
                    }
                    Some(Entry::Clock(s)) => clock = Some(s),
                    Some(Entry::Join { account, token }) => joins.push((account, token)),
                    None => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad journal line {}: {text:?}", entries + 1))),
                }
                entries += 1;
                good_len = end as u64 + 1;
            }
        }
        // drop any torn tail so new entries start on a clean line
        let f = OpenOptions::new().create(true).write(true).truncate(false).open(path)?;
        f.set_len(good_len)?;
        let mut f = OpenOptions::new().append(true).open(path)?;
        f.flush()?;
        Ok((Journal { out: BufWriter::with_capacity(1 << 16, f), entries }, x, clock, joins))
    }

    pub fn append(&mut self, e: &Entry) -> std::io::Result<()> {
        self.out.write_all(encode(e).as_bytes())?;
        self.out.write_all(b"\n")?;
        self.entries += 1;
        Ok(())
    }

    /// Flush buffered entries and make them durable. Called once per batch
    /// (group commit), before any reply in that batch is sent.
    pub fn commit(&mut self) -> std::io::Result<()> {
        self.out.flush()?;
        self.out.get_ref().sync_data()
    }
}
