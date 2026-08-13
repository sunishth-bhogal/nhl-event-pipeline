import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import type { NhlEvent } from "@nhl/schema";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPLAYS_DIR = path.resolve(__dirname, "../../../data/replays");

// Cap the real-time wait between consecutive events so intermissions (which
// can be many minutes of game-clock) don't stall a live demo — the events'
// gameClockMs (their true event time) is unaffected by this cap, only how
// fast we play them back.
const MAX_INTER_EVENT_WAIT_MS = 4000;

export function loadFixture(fixture: string): NhlEvent[] {
  const filePath = path.join(REPLAYS_DIR, fixture);
  const raw = readFileSync(filePath, "utf-8");
  return raw
    .split("\n")
    .filter((line) => line.trim().length > 0)
    .map((line) => JSON.parse(line) as NhlEvent);
}

/** Async generator that yields events from a fixture, pacing emission by the
 * gap between consecutive events' gameClockMs (event time), scaled by `speed`.
 * Aborts promptly when `signal` fires so switching replay/fixture/live mode
 * from the UI takes effect immediately. */
export async function* replayEvents(
  fixture: string,
  speed: number,
  signal: AbortSignal,
): AsyncGenerator<NhlEvent> {
  const events = loadFixture(fixture);
  let prevClock = events.length ? events[0].gameClockMs : 0;

  for (const event of events) {
    if (signal.aborted) return;
    const gapMs = Math.max(0, event.gameClockMs - prevClock);
    const waitMs = Math.min(gapMs / speed, MAX_INTER_EVENT_WAIT_MS);
    if (waitMs > 0) await sleep(waitMs, signal);
    if (signal.aborted) return;
    prevClock = event.gameClockMs;
    yield event;
  }
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const t = setTimeout(resolve, ms);
    signal.addEventListener("abort", () => {
      clearTimeout(t);
      resolve();
    });
  });
}
