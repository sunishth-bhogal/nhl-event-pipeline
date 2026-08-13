/** Shared mapping from raw NHL gamecenter play-by-play JSON to our canonical
 * NhlEvent shape. Used by both the offline fixture recorder and the live poller
 * so the two sources produce byte-for-byte comparable events. */
import { v4 as uuidv4, v5 as uuidv5 } from "uuid";
import { toGameClockMs, type NhlEvent, type PlayerRef, EventTypeSchema } from "@nhl/schema";

// Fixed namespace so live mode can derive a stable eventId per (gameId, NHL play id) —
// re-polling the same play twice naturally yields the same eventId instead of a fresh
// uuid, so only the Failure Lab (not our own polling) is a source of duplicates.
const LIVE_EVENT_NAMESPACE = "6f1a6f2e-6e33-4b7a-9a9a-1a9d3e2c8b10";

export type RosterMap = Map<number, { name: string; teamId: number }>;

export function buildRosterMap(rosterSpots: any[]): RosterMap {
  const map: RosterMap = new Map();
  for (const spot of rosterSpots ?? []) {
    map.set(spot.playerId, {
      name: `${spot.firstName?.default ?? ""} ${spot.lastName?.default ?? ""}`.trim(),
      teamId: spot.teamId,
    });
  }
  return map;
}

function playerRef(roster: RosterMap, playerId: number | undefined, role: string): PlayerRef | null {
  if (!playerId) return null;
  const info = roster.get(playerId);
  return { id: playerId, name: info?.name ?? `Player ${playerId}`, role };
}

function extractPlayers(typeDescKey: string, details: any, roster: RosterMap): PlayerRef[] {
  const out: (PlayerRef | null)[] = [];
  switch (typeDescKey) {
    case "goal":
      out.push(
        playerRef(roster, details.scoringPlayerId, "scorer"),
        playerRef(roster, details.assist1PlayerId, "assist"),
        playerRef(roster, details.assist2PlayerId, "assist"),
        playerRef(roster, details.goalieInNetId, "goalie"),
      );
      break;
    case "shot-on-goal":
      out.push(
        playerRef(roster, details.shootingPlayerId, "shooter"),
        playerRef(roster, details.goalieInNetId, "goalie"),
      );
      break;
    case "missed-shot":
      out.push(playerRef(roster, details.shootingPlayerId, "shooter"));
      break;
    case "blocked-shot":
      out.push(
        playerRef(roster, details.shootingPlayerId, "shooter"),
        playerRef(roster, details.blockingPlayerId, "blocker"),
      );
      break;
    case "penalty":
    case "delayed-penalty":
      out.push(
        playerRef(roster, details.committedByPlayerId, "penalized"),
        playerRef(roster, details.drawnByPlayerId, "drawn-by"),
      );
      break;
    case "faceoff":
      out.push(
        playerRef(roster, details.winningPlayerId, "faceoff-winner"),
        playerRef(roster, details.losingPlayerId, "faceoff-loser"),
      );
      break;
    case "hit":
      out.push(
        playerRef(roster, details.hittingPlayerId, "hitter"),
        playerRef(roster, details.hitteePlayerId, "hittee"),
      );
      break;
    case "giveaway":
    case "takeaway":
      out.push(playerRef(roster, details.playerId, typeDescKey === "giveaway" ? "giveaway" : "takeaway"));
      break;
  }
  return out.filter((p): p is PlayerRef => p !== null);
}

function describe(typeDescKey: string, players: PlayerRef[], details: any): string {
  switch (typeDescKey) {
    case "goal": {
      const scorer = players.find((p) => p.role === "scorer");
      const assists = players.filter((p) => p.role === "assist");
      const assistText = assists.length ? ` (assists: ${assists.map((a) => a.name).join(", ")})` : " (unassisted)";
      return `GOAL: ${scorer?.name ?? "Unknown"}${assistText}`;
    }
    case "penalty":
      return `Penalty: ${players.find((p) => p.role === "penalized")?.name ?? "Unknown"}`;
    case "hit": {
      const hitter = players.find((p) => p.role === "hitter");
      const hittee = players.find((p) => p.role === "hittee");
      return `Hit: ${hitter?.name ?? "?"} on ${hittee?.name ?? "?"}`;
    }
    case "faceoff": {
      const win = players.find((p) => p.role === "faceoff-winner");
      return `Faceoff won by ${win?.name ?? "?"}`;
    }
    case "shot-on-goal":
      return `Shot on goal: ${players.find((p) => p.role === "shooter")?.name ?? "?"}`;
    case "period-start":
      return "Period start";
    case "period-end":
      return "Period end";
    case "game-end":
      return "Game end";
    case "stoppage":
      return `Stoppage${details.reason ? `: ${details.reason}` : ""}`;
    default:
      return typeDescKey;
  }
}

export function teamsById(data: any): Map<number, { id: number; abbrev: string }> {
  const map = new Map<number, { id: number; abbrev: string }>();
  for (const t of [data.awayTeam, data.homeTeam]) {
    if (t) map.set(t.id, { id: t.id, abbrev: t.abbrev });
  }
  return map;
}

/** mode "fixture" assigns fresh random eventIds (used once, offline, for the static
 * replay file); mode "live" derives a stable eventId per NHL play id so repeated
 * polling of the same game doesn't mint new ids for plays we've already seen. */
export function mapPlaysToEvents(data: any, mode: "fixture" | "live"): NhlEvent[] {
  const roster = buildRosterMap(data.rosterSpots);
  const teams = teamsById(data);
  const gameId = String(data.id);

  const events: NhlEvent[] = [];
  let seq = 0;
  for (const play of data.plays ?? []) {
    const parsed = EventTypeSchema.safeParse(play.typeDescKey);
    if (!parsed.success) continue;
    const details = play.details ?? {};
    const players = extractPlayers(play.typeDescKey, details, roster);
    const team = details.eventOwnerTeamId ? teams.get(details.eventOwnerTeamId) ?? null : null;
    const period = play.periodDescriptor?.number ?? 1;
    const timeRemaining = play.timeRemaining ?? "20:00";

    const eventId =
      mode === "live" ? uuidv5(`${gameId}:${play.eventId}`, LIVE_EVENT_NAMESPACE) : uuidv4();

    events.push({
      eventId,
      gameId,
      sequenceNumber: seq++,
      eventType: parsed.data,
      period,
      periodTimeRemaining: timeRemaining,
      gameClockMs: toGameClockMs(period, timeRemaining),
      team: team ? { id: team.id, abbrev: team.abbrev } : null,
      players,
      description: describe(play.typeDescKey, players, details),
      details,
      source: mode === "live" ? "live" : "replay",
    });
  }

  events.sort((a, b) => a.gameClockMs - b.gameClockMs || a.sequenceNumber - b.sequenceNumber);
  return events;
}
