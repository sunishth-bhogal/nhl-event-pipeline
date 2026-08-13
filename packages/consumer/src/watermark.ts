import { EventEmitter } from "node:events";
import type { NhlEvent, EventEnvelope, ActivityEntry } from "@nhl/schema";
import { markSeenIfNew, insertPlayStmt, insertDiscardedStmt } from "./db.js";
import { metrics } from "./metrics.js";

// All three bounds are in game-clock ms (event time), not wall-clock — event
// time is invariant to replay speed, so these thresholds behave consistently
// whether you're replaying at 1x or 20x.
const ALLOWED_LATENESS_MS = 3000; // soft watermark: window before finalization
const HARD_LATENESS_BOUND_MS = 12000; // beyond this, too late to usefully reorder
const FORCE_FLUSH_AFTER_MS = 6000; // wall-clock safety valve so a stalled game doesn't wedge the buffer
const FLUSH_INTERVAL_MS = 500;
// A sequence number is only declared "confirmed missing" once this many later
// sequence numbers have arrived — comfortably past the max reorder window (10)
// and typical late-arrival distance, so genuine drops are distinguished from
// events that are merely reordered or artificially delayed, not gone.
const GAP_CONFIRM_DISTANCE = 20;

interface Buffered {
  event: NhlEvent;
  producedAt: number;
  bufferedAt: number;
  wasLate: boolean;
  lateByMs: number;
}

interface GameState {
  maxGameClockSeen: number;
  seenSequences: Set<number>;
  maxSequenceSeen: number;
  reportedGaps: Set<number>;
  buffer: Buffered[]; // kept sorted by gameClockMs
}

export const activityBus = new EventEmitter();

const games = new Map<string, GameState>();

function getGame(gameId: string): GameState {
  let g = games.get(gameId);
  if (!g) {
    g = {
      maxGameClockSeen: 0,
      seenSequences: new Set(),
      maxSequenceSeen: -1,
      reportedGaps: new Set(),
      buffer: [],
    };
    games.set(gameId, g);
  }
  return g;
}

function insertSorted(buffer: Buffered[], item: Buffered) {
  let i = buffer.length;
  while (i > 0 && buffer[i - 1].event.gameClockMs > item.event.gameClockMs) i--;
  buffer.splice(i, 0, item);
}

function emit(entry: ActivityEntry) {
  activityBus.emit("activity", entry);
}

function finalize(gameId: string, item: Buffered) {
  const { event, producedAt, wasLate, lateByMs } = item;
  const processedAt = Date.now();
  insertPlayStmt.run({
    eventId: event.eventId,
    gameId,
    sequenceNumber: event.sequenceNumber,
    eventType: event.eventType,
    period: event.period,
    periodTimeRemaining: event.periodTimeRemaining,
    gameClockMs: event.gameClockMs,
    teamAbbrev: event.team?.abbrev ?? null,
    description: event.description,
    playersJson: JSON.stringify(event.players),
    producedAt,
    processedAt,
    wasLate: wasLate ? 1 : 0,
    lateByMs,
    source: event.source,
  });
  metrics.recordIngested(processedAt - producedAt);
  if (wasLate) metrics.recordLateRecovered();
  emit({
    id: `${event.eventId}:${processedAt}`,
    kind: wasLate ? "late-recovered" : "ingested",
    gameId,
    eventId: event.eventId,
    eventType: event.eventType,
    description: wasLate ? `Recovered ${lateByMs}ms late: ${event.description}` : event.description,
    gameClockMs: event.gameClockMs,
    processedAt,
    lateByMs: wasLate ? lateByMs : undefined,
  });
}

function sweepGaps(game: GameState, gameId: string, force: boolean) {
  const threshold = force ? game.maxSequenceSeen : game.maxSequenceSeen - GAP_CONFIRM_DISTANCE;
  for (let seq = 0; seq <= threshold; seq++) {
    if (game.seenSequences.has(seq) || game.reportedGaps.has(seq)) continue;
    game.reportedGaps.add(seq);
    metrics.recordGapDetected();
    emit({
      id: `${gameId}:gap:${seq}`,
      kind: "gap-detected",
      gameId,
      eventId: `missing-seq-${seq}`,
      eventType: "unknown",
      description: `Gap detected: sequence #${seq} never arrived (likely dropped upstream)`,
      gameClockMs: 0,
      processedAt: Date.now(),
    });
  }
}

export function processEnvelope(envelope: EventEnvelope) {
  const { event, producedAt } = envelope;
  const gameId = event.gameId;

  const isNew = markSeenIfNew(event.eventId, gameId);
  if (!isNew) {
    metrics.recordDuplicate();
    emit({
      id: `${event.eventId}:${Date.now()}:dup`,
      kind: "duplicate-suppressed",
      gameId,
      eventId: event.eventId,
      eventType: event.eventType,
      description: `Duplicate suppressed: ${event.description}`,
      gameClockMs: event.gameClockMs,
      processedAt: Date.now(),
    });
    return;
  }

  const game = getGame(gameId);
  game.seenSequences.add(event.sequenceNumber);
  game.maxSequenceSeen = Math.max(game.maxSequenceSeen, event.sequenceNumber);

  const softWatermark = game.maxGameClockSeen - ALLOWED_LATENESS_MS;
  const hardWatermark = game.maxGameClockSeen - HARD_LATENESS_BOUND_MS;
  if (event.gameClockMs > game.maxGameClockSeen) game.maxGameClockSeen = event.gameClockMs;

  if (event.gameClockMs < hardWatermark) {
    metrics.recordLateDiscarded();
    const lateByMs = hardWatermark - event.gameClockMs;
    insertDiscardedStmt.run({
      eventId: event.eventId,
      gameId,
      eventType: event.eventType,
      gameClockMs: event.gameClockMs,
      lateByMs,
      discardedAt: Date.now(),
    });
    emit({
      id: `${event.eventId}:${Date.now()}:late-discard`,
      kind: "late-discarded",
      gameId,
      eventId: event.eventId,
      eventType: event.eventType,
      description: `Discarded (too late to reorder): ${event.description}`,
      gameClockMs: event.gameClockMs,
      processedAt: Date.now(),
      lateByMs,
    });
    return;
  }

  const wasLate = event.gameClockMs < softWatermark;
  const lateByMs = wasLate ? softWatermark - event.gameClockMs : 0;
  insertSorted(game.buffer, { event, producedAt, bufferedAt: Date.now(), wasLate, lateByMs });

  if (event.eventType === "game-end") flushGame(gameId, true);
}

function flushGame(gameId: string, force: boolean) {
  const game = games.get(gameId);
  if (!game) return;
  const now = Date.now();
  const softWatermark = game.maxGameClockSeen - ALLOWED_LATENESS_MS;

  while (game.buffer.length > 0) {
    const head = game.buffer[0];
    const ready =
      force || head.event.gameClockMs <= softWatermark || now - head.bufferedAt >= FORCE_FLUSH_AFTER_MS;
    if (!ready) break;
    game.buffer.shift();
    finalize(gameId, head);
  }

  sweepGaps(game, gameId, force);
}

export function tickAllGames() {
  for (const gameId of games.keys()) flushGame(gameId, false);
}

export function oldestPendingAgeMs(): number {
  const now = Date.now();
  let max = 0;
  for (const game of games.values()) {
    if (game.buffer.length > 0) max = Math.max(max, now - game.buffer[0].bufferedAt);
  }
  return max;
}

setInterval(tickAllGames, FLUSH_INTERVAL_MS);
