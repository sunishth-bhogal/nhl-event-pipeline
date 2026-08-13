/**
 * The NHL public API gives us period + time-remaining-in-period, not a wall-clock
 * timestamp per play. For a streaming pipeline we need a single monotonic axis to
 * call "event time" — so we use the in-game clock itself (elapsed ms since puck
 * drop) rather than trying to fabricate real-world timestamps. This is the
 * domain's actual event time: two plays with the same gameClockMs happened at the
 * same moment in the game, regardless of when either was ingested.
 */

const PERIOD_LENGTH_MS = 20 * 60 * 1000;
const OT_LENGTH_MS = 5 * 60 * 1000;

export function parseTimeRemaining(mmss: string): number {
  const [min, sec] = mmss.split(":").map((n) => Number.parseInt(n, 10));
  return (min || 0) * 60_000 + (sec || 0) * 1000;
}

/** Elapsed game-time in ms since puck drop of period 1, given period number and time remaining in that period. */
export function toGameClockMs(period: number, periodTimeRemaining: string): number {
  const remainingMs = parseTimeRemaining(periodTimeRemaining);
  let elapsedBeforePeriod = 0;
  for (let p = 1; p < period; p++) {
    elapsedBeforePeriod += p <= 3 ? PERIOD_LENGTH_MS : OT_LENGTH_MS;
  }
  const periodLength = period <= 3 ? PERIOD_LENGTH_MS : OT_LENGTH_MS;
  const elapsedInPeriod = Math.max(0, periodLength - remainingMs);
  return elapsedBeforePeriod + elapsedInPeriod;
}
