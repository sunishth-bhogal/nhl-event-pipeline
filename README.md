# NHL Event Pipeline — Failure Lab

A Kafka-backed streaming pipeline for NHL play-by-play events, built to demonstrate
the parts of distributed systems that a "Kafka + Docker + K8s" checklist project
usually skips: **why** each piece exists, proven by making failures visible and
handled, not just building a happy-path demo.

The core claim this project backs up: **the pipeline is correct under duplicate
delivery, out-of-order arrival, and dropped messages** — not just under normal
conditions. The Failure Lab is the mechanism for proving that live, on demand.

## Architecture

```
                      ┌─────────────────┐
                      │   control-api    │  control plane (state + commands)
                      │  (Express + WS)  │
                      └───────┬──────────┘
                              │ WS: failure-lab toggles, replay/live commands
                              ▼
┌──────────────┐      ┌──────────────┐      ┌──────────────────┐
│  NHL API /   │─────▶│   producer   │─────▶│  Kafka            │
│  fixture     │      │ (Failure Lab │      │  nhl.events.raw   │
│  (NDJSON)    │      │  injection)  │      │  (3 partitions,   │
└──────────────┘      └──────────────┘      │   keyed by gameId)│
                                             └─────────┬─────────┘
                                                        ▼
                                             ┌──────────────────┐
                                             │     consumer      │  data + observability plane
                                             │  dedup, watermark, │  (Express + WS on :4001)
                                             │  gap detection,    │
                                             │  SQLite read-model │
                                             └─────────┬─────────┘
                                                        ▼
                                             ┌──────────────────┐
                                             │       web         │  dashboard (Vite/React, :5173)
                                             └──────────────────┘
```

Three planes, deliberately separated:
- **Control plane** (`control-api`, :4000) — Failure Lab toggles and replay/live
  commands. Independent of Kafka: it stays responsive even if the broker is down.
- **Data plane** (Kafka) — the actual event stream. The only thing the producer
  and consumer share.
- **Observability plane** (`consumer`'s own API, :4001) — metrics, live activity
  log, and the finalized play-by-play read-model, all derived from what the
  consumer has actually processed.

## Why event time ≠ processing time

The NHL API gives plays a period + time-remaining, not a wall-clock timestamp.
Rather than fabricate one, [`gameClockMs`](packages/schema/src/gameClock.ts) —
elapsed milliseconds since puck drop — is used as the **event time** everywhere
in this system. It's the domain's actual notion of "when did this happen,"
invariant to how fast replay is played back or how long a message sits in a
queue. `producedAt` (set by the producer) and `processedAt` (set by the
consumer) are the **processing time** — real wall-clock epoch ms — and are only
ever used to measure pipeline latency, never to order events.

The consumer computes a per-game watermark: `maxEventTimeSeen - 3000ms`. Events
are buffered and only finalized into the read-model once the watermark passes
their `gameClockMs`. An event that arrives after its window closed is still
inserted in correct chronological order (`late-recovered`) as long as it's
within a hard bound (12s of event time); beyond that it's `late-discarded` —
a real production tradeoff between correctness and finalization latency, made
explicit instead of hidden. See [`watermark.ts`](packages/consumer/src/watermark.ts).

## The Failure Lab

Toggled live from the dashboard, applied by the **producer** at publish time
(the one place that can still choose to mess with delivery before it becomes
"real" on the wire):

| Failure | What it does | What proves it's handled |
|---|---|---|
| Duplicate | Re-publishes the same event (same `eventId`) a second time | Consumer's SQLite-backed idempotency check suppresses it — `duplicatesDropped` metric, visible in the activity feed |
| Drop | Never publishes the event at all | Consumer detects the resulting gap in `sequenceNumber` after enough later events arrive to confirm it isn't just reordered — `gapsDetected` |
| Late arrival | Delays publish by a random interval | Consumer's watermark either reorders it into the correct position (`lateRecovered`) or discards it if too late (`lateDiscarded`) |
| Reorder | Buffers N events and shuffles publish order | Same watermark logic reconstructs event-time order regardless of arrival order |

## Replay mode

Live NHL games happen on inconsistent schedules — a demo that only works when
a game happens to be live is a bad demo. `npm run record-fixture` fetches a
completed game's real play-by-play from the NHL public API once, offline, and
freezes it to NDJSON (see [`data/replays/`](data/replays)). Replay mode reads
that file and re-publishes it at a configurable speed (1x–25x), preserving the
original event-time gaps — no network dependency at demo time.

Live mode also exists and works the same way against an in-progress game
(`packages/producer/src/liveSource.ts`), polling every 5s — same schema, same
Failure Lab pipeline, same consumer. The consumer never knows which mode it came from.

## Running it

Requires [Docker Desktop](https://www.docker.com/products/docker-desktop/) (not
installed in the environment this was built in — install it first).

```bash
npm install
npm run kafka:up          # starts Kafka (KRaft mode) + Kafka UI at :8080
npm run dev                # starts control-api, consumer, producer, web together
```

Then open http://localhost:5173, pick the recorded fixture, hit **Start
replay**, and start moving the Failure Lab sliders. Watch the activity feed and
metrics react in real time.

To record a different game as a fixture:
```bash
npm run record-fixture -w @nhl/producer -- <nhlGameId>
```
Find game IDs via `https://api-web.nhle.com/v1/score/<YYYY-MM-DD>`.

## What's actually measured (not invented)

`latencyP50Ms` / `latencyP95Ms` in the metrics bar are real samples of
`processedAt - producedAt` from live traffic — not an assumed number. Before
citing a specific figure (e.g. in a resume bullet), run a fixture at 1x speed
with the Failure Lab at zero and record what the dashboard actually reports;
then run it again with duplicate/late/drop rates turned up and report both.
The interesting number isn't "how fast," it's "how much does correctness cost
under failure" — that's the comparison worth having ready for an interview.

## Build stages

1. **Schema + Kafka topic design** — [`packages/schema`](packages/schema) — done
2. **Producer**: live NHL polling + replay mode + Failure Lab injection — done
3. **Consumer**: idempotent dedup, event-time watermarking, gap detection,
   SQLite read-model — done
4. **Failure Lab UI**: live toggles, activity feed, metrics dashboard — done
5. **Next**: generalize the event schema to a second sport (stripped-down) to
   prove it wasn't NHL-specific — not started. Everything in `packages/schema`
   outside of `nhlMapper.ts`'s field extraction is already sport-agnostic; the
   real test is whether a second producer can reuse `EventEnvelope`,
   `FailureLabState`, and the consumer's watermark/dedup logic unmodified.

## Project layout

```
packages/
  schema/        canonical event types, topics, control/metrics contracts (Zod)
  producer/      live + replay sources, Failure Lab injection, Kafka producer
  consumer/      dedup, watermarking, gap detection, SQLite, observability API
  control-api/   Failure Lab + replay control plane (Express + WS)
  web/           React dashboard
data/replays/    recorded NDJSON fixtures for replay mode
docker-compose.yml   single-node Kafka (KRaft) + Kafka UI
```
