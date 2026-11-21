//! Crash recovery: replaying the journal rebuilds the exact exchange state.

use breakaway_engine::exchange::*;
use breakaway_engine::journal::*;
use breakaway_engine::*;
use std::io::Write;

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

fn tmp(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("breakaway-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_file(&p);
    p
}

fn random_entries(n: usize, seed: u64) -> Vec<Entry> {
    let mut r = Rng(seed);
    let mut v: Vec<Entry> = (0..8).map(|a| Entry::Cmd(ExCommand::Deposit { account: a, cents: 50_000 })).collect();
    let mut fair = 50i64;
    for i in 0..n {
        fair = (fair + r.below(3) as i64 - 1).clamp(5, 95);
        if i % 50 == 0 {
            v.push(Entry::Clock(i as u32));
        }
        let account = r.below(8) as u32;
        v.push(if r.below(4) == 0 {
            Entry::Cmd(ExCommand::Cancel { account, order_id: 1 + r.below(i as u64 + 1) })
        } else {
            let side = if r.below(2) == 0 { Side::Buy } else { Side::Sell };
            let tif = if r.below(5) == 0 { TimeInForce::Ioc } else { TimeInForce::Gtc };
            Entry::Cmd(ExCommand::Place { account, side, price: (fair + r.below(9) as i64 - 4) as u8, qty: 1 + r.below(30) as u32, tif })
        });
    }
    v
}

#[test]
fn encode_decode_roundtrip() {
    let extra = [
        Entry::Cmd(ExCommand::Settle { home_won: true }),
        Entry::Cmd(ExCommand::Settle { home_won: false }),
        Entry::Join { account: 101, token: 0x00ab_cdef_0123_4567 },
    ];
    for e in random_entries(2_000, 7).into_iter().chain(extra) {
        assert_eq!(decode(&encode(&e)), Some(e), "{}", encode(&e));
    }
    assert_eq!(decode("X 1 2"), None);
    assert_eq!(decode("P 1 Q 50 1 G"), None);
}

#[test]
fn restart_rebuilds_identical_state() {
    let path = tmp("restart");
    let entries = random_entries(30_000, 11);
    let mut live = Exchange::new();
    let mut out = Vec::new();
    {
        let (mut j, _, _) = Journal::open(&path).unwrap();
        for (i, e) in entries.iter().enumerate() {
            j.append(e).unwrap();
            if let Entry::Cmd(c) = e {
                live.apply(c, &mut out);
            }
            if i % 100 == 0 {
                j.commit().unwrap();
            }
        }
        j.commit().unwrap();
    }
    let (j, rebuilt, clock) = Journal::open(&path).unwrap();
    assert_eq!(j.entries as usize, entries.len());
    assert_eq!(rebuilt.engine().state_hash(), live.engine().state_hash());
    assert_eq!(rebuilt.accounts(), live.accounts());
    assert_eq!(rebuilt.escrow(), live.escrow());
    assert_eq!(clock, Some(29_950));
    rebuilt.check().unwrap();
}

#[test]
fn torn_last_line_is_ignored_and_journal_continues() {
    let path = tmp("torn");
    let entries = random_entries(500, 3);
    let mut live = Exchange::new();
    let mut out = Vec::new();
    {
        let (mut j, _, _) = Journal::open(&path).unwrap();
        for e in &entries {
            j.append(e).unwrap();
            if let Entry::Cmd(c) = e {
                live.apply(c, &mut out);
            }
        }
        j.commit().unwrap();
    }
    // simulate a crash halfway through writing the next entry
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"P 3 B 5").unwrap();
    let (mut j, x, _) = Journal::open(&path).unwrap();
    assert_eq!(x.engine().state_hash(), live.engine().state_hash(), "torn line must not be applied");
    // and new entries land on a clean line
    let extra = Entry::Cmd(ExCommand::Deposit { account: 1, cents: 5 });
    j.append(&extra).unwrap();
    j.commit().unwrap();
    drop(j);
    let (j2, x2, _) = Journal::open(&path).unwrap();
    assert_eq!(j2.entries as usize, entries.len() + 1);
    assert_eq!(x2.deposits(), live.deposits() + 5);
}

#[test]
fn corrupt_line_in_the_middle_is_an_error() {
    let path = tmp("corrupt");
    std::fs::write(&path, b"D 1 100\nZZZ\nD 2 100\n").unwrap();
    assert!(Journal::open(&path).is_err(), "never silently skip a committed entry");
}
