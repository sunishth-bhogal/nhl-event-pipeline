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

## Accounts, risk checks and settlement (`exchange.rs`)

Play money, in integer cents, fully collateralised:

* Every contract is half of a pair: a YES share ("home wins") plus a NO share
  always pay exactly 100¢ between them.
* **Buying YES at p** costs p. If you already hold NO, the new YES and an old
  NO are redeemed as a pair for 100¢ instead.
* **Selling YES at p** hands over a YES share you hold, or mints a new pair:
  you put 100¢ into escrow, keep the NO share and receive p (net cost 100 - p).
* So escrow always holds exactly 100¢ per outstanding pair, and the exchange
  never has counterparty risk.
* **Pre-trade risk check:** an order is accepted only if it can be paid for
  if it fully fills at its limit. Shares you hold count as cover first; the
  rest is reserved in cash (p per contract to buy, 100 - p to sell). Fills at
  a better price refund the difference immediately.
* **Settlement:** cancel every open order, pay 100¢ per winning share out of
  escrow, and the losing shares expire.

`Exchange::check()` verifies, and the tests assert after every command:

* money is conserved: all cash (free + reserved) + escrow = total deposits
* escrow = 100¢ x YES outstanding = 100¢ x NO outstanding
* nothing is ever negative
* reserved cash and shares equal exactly what the open orders need, and every
  open order's remaining quantity equals what rests in the book

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
* **Exchange:** hand-checked money scenarios (minting, selling shares you
  own, redeeming pairs, price improvement, risk rejects, settlement) and a
  randomized run of ~580,000 commands across 30 markets that each settle at
  the end, with every invariant checked after every command and each market
  rebuilt from its log. The run produces ~150,000 fills and exercises minting,
  redeeming and refusals for lack of funds.
* **The tests catch real bugs.** Planted mistakes in a copy of the code made
  the suite fail every time: trading at the taker's price, serving a level
  last-in-first-out, leaving an emptied level visible, skipping a
  price-improvement refund, ignoring redemptions in escrow, and reserving 1¢
  too little for a sell.

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
| Latency p99.99 | 2.1–2.5 µs |
| **With accounts, risk checks and escrow** | |
| Throughput | ~25 million commands/sec |
| Latency p50 / p99 / p99.9 | 37 ns / 139 ns / 1.2 µs |

Latencies have the timer's own cost (~21 ns) subtracted. The risk layer
roughly halves throughput (every fill updates two accounts and the escrow);
the benchmark checks it produces the exact same matches as the bare engine.
No network or persistence yet.

## Simulated markets (`src/bin/simulate.rs`)

`cargo run --release --bin simulate -- data/fair ../web/markets`

For each featured replay, runs a market through the full exchange on the
game's second-by-second fair price (from the win-probability model):

* **Market maker** (account 0) sees the fair price 3 seconds late, requotes
  every 2 seconds at two levels a side, widens its spread with recent
  volatility and shades its quotes against its inventory. It stops quoting
  once a game is all but decided.
* **Sharp traders** see the true price immediately and take any quote more than
  2¢ off, mostly in the seconds after a goal, which is where the market maker
  loses money.
* **Noise traders** trade on a rough guess (fair ± ~6¢): they take quotes their
  guess is through, otherwise post limit orders that expire after 1–5 minutes.

Each market settles at the final whistle; the run asserts every exchange
invariant and that P&L across all participants sums to zero. Seeded by game
id, so output is identical every run. Across the 11 featured games: about
1,492 trades and 10,432 contracts per game, the market's midpoint typically within
~2¢ of the model with a 3–4¢ spread. In comeback games the sharps, who trade
on the model, lose: the model was confidently wrong about who would win.
