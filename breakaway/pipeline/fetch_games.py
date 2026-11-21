"""Download NHL play-by-play and store each game in a compact replay format.

Runs in GitHub Actions (this repo's workspace has no direct internet access).
Uses only the standard library.

Usage:
    python pipeline/fetch_games.py 20212022 20222023 20232024 20242025 20252026

Output: data/games-<season>.jsonl.gz, one compact game per line (format below),
plus data/manifest.json with counts.

The NHL "api-web.nhle.com" API is public but undocumented. We go gently:
a handful of parallel requests, retries with backoff, and a User-Agent.

Compact game format
-------------------
{
  "id": 2024021296, "season": 20242025, "type": 2 (regular) | 3 (playoffs),
  "date": "2025-04-14",
  "home": {"id": 2, "abbrev": "NYI", "name": "New York Islanders"},
  "away": {...},
  "final": {"home": 2, "away": 4, "last": "REG" | "OT" | "SO"},
  "plays": [[sec, period, kind, owner, situation, homeScore, awayScore, x, y, note], ...]
}
  sec        seconds since puck drop (OT/SO continue past 3600)
  kind       short event code, see KINDS
  owner      1 = home team's event, 0 = away team's, -1 = neither
  situation  NHL situationCode "AGSSHG" style string, e.g. "1551":
             away goalie, away skaters, home skaters, home goalie
  note       scorer/penalty text for the replay feed ("" otherwise)
"""
from __future__ import annotations

import gzip
import json
import os
import sys
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor

API = "https://api-web.nhle.com/v1/gamecenter/{gid}/play-by-play"
UA = "breakaway-research/0.1 (github.com; educational, low-rate)"
WORKERS = 6

KINDS = {
    "goal": "G",
    "shot-on-goal": "S",
    "missed-shot": "M",
    "blocked-shot": "B",
    "penalty": "P",
    "hit": "H",
    "giveaway": "GV",
    "takeaway": "TK",
    "faceoff": "F",
    "period-start": "PS",
    "period-end": "PE",
    "game-end": "END",
    "shootout-complete": "SOC",
}


def fetch(gid: int) -> dict | None:
    url = API.format(gid=gid)
    for attempt in range(6):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "application/json"})
            with urllib.request.urlopen(req, timeout=30) as r:
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
            if e.code in (429, 500, 502, 503, 504):
                time.sleep(2 ** attempt)
                continue
            return None
        except Exception:
            time.sleep(2 ** attempt)
    return None


def mmss(s: str) -> int:
    m, sec = s.split(":")
    return int(m) * 60 + int(sec)


def game_seconds(period: int, period_type: str, time_in_period: str, game_type: int) -> int:
    t = mmss(time_in_period)
    if period <= 3:
        return (period - 1) * 1200 + t
    if period_type == "SO":
        return 3600 + (300 if game_type == 2 else 0)
    # regular-season OT is one 5-minute period; playoff OTs are 20 minutes each
    return 3600 + (period - 4) * 1200 + t


def names(roster: dict[int, str], *pids) -> list[str]:
    return [roster[p] for p in pids if p and p in roster]


def compact(raw: dict) -> dict | None:
    if raw.get("gameState") not in ("OFF", "FINAL"):
        return None  # only finished games
    gtype = raw.get("gameType")
    if gtype not in (2, 3):
        return None
    home, away = raw["homeTeam"], raw["awayTeam"]
    hid = home["id"]
    roster = {
        r["playerId"]: f'{r["firstName"]["default"][0]}. {r["lastName"]["default"]}'
        for r in raw.get("rosterSpots", [])
    }

    def team(t):
        name = f'{t.get("placeName", {}).get("default", "")} {t.get("commonName", {}).get("default", "")}'.strip()
        return {"id": t["id"], "abbrev": t["abbrev"], "name": name}

    plays = []
    for p in raw.get("plays", []):
        kind = KINDS.get(p.get("typeDescKey"))
        if not kind:
            continue
        pd = p.get("periodDescriptor", {})
        period = pd.get("number", 1)
        ptype = pd.get("periodType", "REG")
        sec = game_seconds(period, ptype, p.get("timeInPeriod", "00:00"), gtype)
        d = p.get("details", {}) or {}
        owner_id = d.get("eventOwnerTeamId")
        owner = -1 if owner_id is None else (1 if owner_id == hid else 0)
        note = ""
        if kind == "G":
            scorer = names(roster, d.get("scoringPlayerId"))
            assists = names(roster, d.get("assist1PlayerId"), d.get("assist2PlayerId"))
            note = (scorer[0] if scorer else "Goal") + (f" ({', '.join(assists)})" if assists else "")
        elif kind == "P":
            who = names(roster, d.get("committedByPlayerId"))
            note = f'{who[0] + " · " if who else ""}{d.get("descKey", "penalty").replace("-", " ")} ({d.get("duration", 2)} min)'
        plays.append([
            sec, period, kind, owner, p.get("situationCode", ""),
            d.get("homeScore"), d.get("awayScore"),
            d.get("xCoord"), d.get("yCoord"), note,
        ])

    # carry the running score forward (NHL only attaches scores to goal events)
    hs = as_ = 0
    for row in plays:
        if row[5] is None:
            row[5], row[6] = hs, as_
        else:
            hs, as_ = row[5], row[6]

    return {
        "id": raw["id"],
        "season": raw.get("season"),
        "type": gtype,
        "date": raw.get("gameDate"),
        "home": team(home),
        "away": team(away),
        "final": {
            "home": home.get("score"),
            "away": away.get("score"),
            "last": (raw.get("gameOutcome") or {}).get("lastPeriodType", "REG"),
        },
        "plays": plays,
    }


def season_ids(season: int) -> list[int]:
    start = season // 10000
    ids = [int(f"{start}02{n:04d}") for n in range(1, 1313)]
    for rnd, series in ((1, 8), (2, 4), (3, 2), (4, 1)):
        for s in range(1, series + 1):
            for g in range(1, 8):
                ids.append(int(f"{start}030{rnd}{s}{g}"))
    return ids


def run_season(season: int) -> int:
    ids = season_ids(season)
    out_path = f"data/games-{season}.jsonl.gz"
    games = []
    misses = 0
    with ThreadPoolExecutor(WORKERS) as pool:
        for i, raw in enumerate(pool.map(fetch, ids)):
            gid = ids[i]
            if raw is None:
                misses += 1  # not played yet, or a playoff game that wasn't needed
                continue
            g = compact(raw)
            if g:
                games.append(g)
            if i % 200 == 0:
                print(f"{season}: {i}/{len(ids)} fetched, {len(games)} finished games", flush=True)
    games.sort(key=lambda g: (g["date"] or "", g["id"]))
    with gzip.open(out_path, "wt", encoding="utf-8") as f:
        for g in games:
            f.write(json.dumps(g, separators=(",", ":")) + "\n")
    print(f"{season}: wrote {len(games)} games to {out_path} ({misses} ids not found)", flush=True)
    return len(games)


def main(argv: list[str]) -> None:
    os.makedirs("data", exist_ok=True)
    manifest_path = "data/manifest.json"
    manifest = json.load(open(manifest_path)) if os.path.exists(manifest_path) else {}
    for s in argv:
        manifest[s] = {"games": run_season(int(s)), "fetched": time.strftime("%Y-%m-%d")}
    json.dump(manifest, open(manifest_path, "w"), indent=2)


if __name__ == "__main__":
    main(sys.argv[1:] or ["20242025"])
