import Database from "better-sqlite3";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const DB_PATH = process.env.CONSUMER_DB_PATH ?? path.resolve(__dirname, "../../../data/readmodel.sqlite");

export const db: Database.Database = new Database(DB_PATH);
db.pragma("journal_mode = WAL");

db.exec(`
  CREATE TABLE IF NOT EXISTS seen_events (
    event_id TEXT PRIMARY KEY,
    game_id TEXT NOT NULL,
    first_seen_at INTEGER NOT NULL
  );

  CREATE TABLE IF NOT EXISTS plays (
    event_id TEXT PRIMARY KEY,
    game_id TEXT NOT NULL,
    sequence_number INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    period INTEGER NOT NULL,
    period_time_remaining TEXT NOT NULL,
    game_clock_ms INTEGER NOT NULL,
    team_abbrev TEXT,
    description TEXT NOT NULL,
    players_json TEXT NOT NULL,
    produced_at INTEGER NOT NULL,
    processed_at INTEGER NOT NULL,
    was_late INTEGER NOT NULL DEFAULT 0,
    late_by_ms INTEGER NOT NULL DEFAULT 0,
    source TEXT NOT NULL
  );
  CREATE INDEX IF NOT EXISTS idx_plays_game_clock ON plays (game_id, game_clock_ms);

  CREATE TABLE IF NOT EXISTS discarded_late_events (
    event_id TEXT PRIMARY KEY,
    game_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    game_clock_ms INTEGER NOT NULL,
    late_by_ms INTEGER NOT NULL,
    discarded_at INTEGER NOT NULL
  );
`);

export function resetGameData(gameId: string) {
  db.prepare("DELETE FROM plays WHERE game_id = ?").run(gameId);
  db.prepare("DELETE FROM seen_events WHERE game_id = ?").run(gameId);
  db.prepare("DELETE FROM discarded_late_events WHERE game_id = ?").run(gameId);
}

const insertSeenStmt = db.prepare(
  "INSERT OR IGNORE INTO seen_events (event_id, game_id, first_seen_at) VALUES (?, ?, ?)",
);
const hasSeenStmt = db.prepare("SELECT 1 FROM seen_events WHERE event_id = ?");

/** Returns true if this is the first time we've seen this eventId (and records
 * it); false if it's a duplicate. Backed by SQLite (not just an in-memory Set)
 * so idempotency survives a consumer restart — the whole point of using the
 * event's own id as the dedup key instead of relying on in-flight state. */
export function markSeenIfNew(eventId: string, gameId: string): boolean {
  const existing = hasSeenStmt.get(eventId);
  if (existing) return false;
  insertSeenStmt.run(eventId, gameId, Date.now());
  return true;
}

export const insertPlayStmt: Database.Statement = db.prepare(`
  INSERT OR REPLACE INTO plays
    (event_id, game_id, sequence_number, event_type, period, period_time_remaining,
     game_clock_ms, team_abbrev, description, players_json, produced_at, processed_at,
     was_late, late_by_ms, source)
  VALUES (@eventId, @gameId, @sequenceNumber, @eventType, @period, @periodTimeRemaining,
          @gameClockMs, @teamAbbrev, @description, @playersJson, @producedAt, @processedAt,
          @wasLate, @lateByMs, @source)
`);

export const insertDiscardedStmt: Database.Statement = db.prepare(`
  INSERT OR REPLACE INTO discarded_late_events
    (event_id, game_id, event_type, game_clock_ms, late_by_ms, discarded_at)
  VALUES (@eventId, @gameId, @eventType, @gameClockMs, @lateByMs, @discardedAt)
`);

export function getPlays(gameId: string) {
  return db
    .prepare(
      "SELECT * FROM plays WHERE game_id = ? ORDER BY game_clock_ms ASC, sequence_number ASC",
    )
    .all(gameId);
}
