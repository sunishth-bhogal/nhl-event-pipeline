import type { NhlEvent } from "@nhl/schema";
import { mapPlaysToEvents } from "./nhlMapper.js";

const POLL_INTERVAL_MS = 5000;

/** Polls the NHL gamecenter endpoint for a live/in-progress game and yields
 * only plays we haven't seen yet (by our derived stable eventId). This is the
 * real, non-simulated path — same schema and Failure Lab pipeline as replay
 * mode downstream, so the consumer never has to know which source it's from. */
export async function* liveEvents(gameId: string, signal: AbortSignal): AsyncGenerator<NhlEvent> {
  const seen = new Set<string>();

  while (!signal.aborted) {
    try {
      const res = await fetch(`https://api-web.nhle.com/v1/gamecenter/${gameId}/play-by-play`);
      if (res.ok) {
        const data = await res.json();
        const events = mapPlaysToEvents(data, "live");
        for (const event of events) {
          if (!seen.has(event.eventId)) {
            seen.add(event.eventId);
            yield event;
          }
        }
      } else {
        console.warn(`[producer] live fetch for game ${gameId} returned ${res.status}`);
      }
    } catch (err) {
      console.error("[producer] live fetch failed", err);
    }
    await sleep(POLL_INTERVAL_MS, signal);
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
