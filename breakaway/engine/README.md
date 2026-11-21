# breakaway-engine

The matching engine behind Breakaway's markets. One order book per game for a
binary contract, **"the home team wins"**, priced 1–99¢ and settling at 100¢
if the home team wins, so a price is a win probability. Zero dependencies.

## Rules

* **Price-time priority:** best price first, then the earliest order at that price.
* **Trades at the resting order's price.**
* **Good-'til-cancelled** orders rest; **immediate-or-cancel** orders never do.
* **Self-trade prevention:** an order never fills against its own account; the
  resting order is cancelled instead.
* **Deterministic:** the same commands always give the same events and book,
  so a book can be rebuilt exactly by replaying its command log.

## Design

* 99 price levels per side. Each level is a FIFO queue threaded through a slab
  of order nodes (an intrusive doubly linked list), so placing, cancelling and
  filling are O(1) and allocation-free once warmed up.
* A 128-bit mask per side marks non-empty levels. The best bid is the highest
  set bit and the best ask the lowest, so the top of book is one instruction.
* Order ids are dense, so id → slot is a plain array lookup, not a hash map.

## How it's tested

`cargo test --release`

* **Rule tests** for priority, partial fills, IOC, self-trade prevention,
  cancels and input validation.
* **Model-based testing against a reference book.** A deliberately naive
  implementation (a flat list scanned in full on every match) receives the same
  random order flow, including invalid input, wrong-owner cancels and cancels
  of orders that no longer exist. Every event must match exactly, and after
  every command the book must be uncrossed, every price level's quantity must
  match, and each order's quantity must be fully accounted for (filled, rested
  or cancelled). Default run: 1M commands; `BREAKAWAY_SEEDS=400` runs 10M.
* **Replay:** rebuilding a book from a 200,000-command log reproduces the same
  events, snapshot and state hash.
* **The tests catch real bugs.** Planting three mistakes in a copy of the
  engine (trading at the taker's price, serving a level last-in-first-out,
  leaving an emptied level visible) made the suite fail each time.

## Performance

`cargo run --release --bin bench`

Realistic order flow (a drifting fair price, ~61% resting limits, ~6% IOC,
~33% cancels, ~7,000 resting orders), recorded once and replayed into a fresh
engine so random-number generation isn't timed. Single thread, measured in a
shared 2-vCPU cloud VM (Intel Xeon), so treat the tail as noisy:

| | |
|---|---:|
| Throughput | ~57 million commands/sec |
| Latency p50 | 19 ns |
| Latency p99 | 64 ns |
| Latency p99.9 | 191 ns |
| Latency p99.99 | 2.5 µs |

Latencies have the timer's own cost (~21 ns) subtracted. These are engine-only
numbers: no network, persistence or risk checks yet; those come next and will
be measured separately.
