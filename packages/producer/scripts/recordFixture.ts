/**
 * Fetches a real, completed game's play-by-play from the NHL public API and
 * freezes it into an NDJSON fixture (one NhlEvent per line, in event-time
 * order) for replay mode. Recording is a one-time offline step — replay mode
 * itself never touches the network, which is the point: your demo shouldn't
 * depend on a game happening to be live when someone's interviewing you.
 *
 * Usage: npm run record-fixture -w @nhl/producer -- <gameId>
 */
import { writeFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { mapPlaysToEvents } from "../src/nhlMapper.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const OUT_DIR = path.resolve(__dirname, "../../../data/replays");

const gameId = process.argv[2] ?? "2024021296"; // WSH @ NYI, 2025-04-15

async function main() {
  console.log(`Fetching play-by-play for game ${gameId}...`);
  const res = await fetch(`https://api-web.nhle.com/v1/gamecenter/${gameId}/play-by-play`);
  if (!res.ok) throw new Error(`NHL API returned ${res.status}`);
  const data: any = await res.json();

  const events = mapPlaysToEvents(data, "fixture");

  mkdirSync(OUT_DIR, { recursive: true });
  const outPath = path.join(OUT_DIR, `${gameId}.ndjson`);
  writeFileSync(outPath, events.map((e) => JSON.stringify(e)).join("\n") + "\n");

  console.log(`Wrote ${events.length} events to ${outPath}`);
  console.log(`Matchup: ${data.awayTeam.abbrev} @ ${data.homeTeam.abbrev} — ${data.gameDate}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
