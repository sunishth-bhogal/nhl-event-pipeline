"""Pick featured games and write replay files for the web player.

    python model/export_games.py      # after model/train.py

Writes web/games/<id>.json (one per featured game), web/games/index.json,
and copies the model + report into web/ so the page is self-contained.

Every game gets the same treatment: pregame ratings use only earlier games,
and the probabilities shown are computed by the shipped model.
"""
from __future__ import annotations

import json
import os
import shutil
import sys

sys.path.insert(0, os.path.dirname(__file__))
from train import ROOT, attach_elo, load_games  # noqa: E402
from wp import predict  # noqa: E402

DROP = {"F", "H", "GV", "TK"}  # not used by the player; keeps files small


def series(model: dict, g: dict) -> list[tuple[float, float]]:
    """P(home wins) after each event, as (sec, p)."""
    out = [(0, predict(model, 0, 0, 0, "1551", g["elo_diff"]))]
    for p in g["plays"]:
        sec, period, kind, _, sit, hs, as_ = p[:7]
        shootout = g["type"] == 2 and period >= 5
        final = kind in ("END", "SOC")
        if final:
            hs, as_ = g["final"]["home"], g["final"]["away"]
        out.append((sec, predict(model, sec, hs, as_, sit, g["elo_diff"], shootout, final)))
    return out


def drama(model: dict, g: dict) -> dict:
    s = series(model, g)
    home_won = g["final"]["home"] > g["final"]["away"]
    winner_low = min(p if home_won else 1 - p for _, p in s[:-1])
    swing = sum(abs(b[1] - a[1]) for a, b in zip(s, s[1:-1]))
    return {"winner_low": winner_low, "swing": swing}


def replay_file(g: dict) -> dict:
    """[sec, period, kind, owner, situation, homeScore, awayScore, note] per event."""
    plays = [p[:7] + [p[9]] for p in g["plays"] if p[2] not in DROP]
    return {k: g[k] for k in ("id", "season", "type", "date", "home", "away", "final")} | {
        "elo_diff": round(g["elo_diff"], 2),
        "plays": plays,
    }


def label(g: dict) -> str:
    kind = "Playoffs" if g["type"] == 3 else "Regular season"
    return f'{g["away"]["abbrev"]} @ {g["home"]["abbrev"]} · {g["date"]} · {kind}'


def main():
    model = json.load(open(os.path.join(ROOT, "model", "wp_model.json")))
    games = load_games()
    attach_elo(games)
    seasons = sorted({g["season"] for g in games})
    recent = [g for g in games if g["season"] in seasons[-2:]]
    scored = [(g, drama(model, g)) for g in recent]

    picks: list[tuple[dict, str]] = []
    seen: set[int] = set()

    def add(g, why):
        if g["id"] not in seen and len(g["plays"]) > 5:
            seen.add(g["id"])
            picks.append((g, why))

    finals = [g for g in recent if g["type"] == 3 and str(g["id"])[6:8] == "04"]
    if finals:
        add(max(finals, key=lambda g: g["id"]), "Stanley Cup clincher")
    for g, d in sorted(scored, key=lambda gd: gd[1]["winner_low"])[:4]:
        who = g["home"]["abbrev"] if g["final"]["home"] > g["final"]["away"] else g["away"]["abbrev"]
        add(g, f"Comeback: {who} won after the model gave them {d['winner_low'] * 100:.0f}%")
    for g, d in sorted(scored, key=lambda gd: -gd[1]["swing"])[:3]:
        add(g, "One of the wildest games: biggest total swings")
    leafs = [(g, d) for g, d in scored if "TOR" in (g["home"]["abbrev"], g["away"]["abbrev"])]
    for g, d in sorted(leafs, key=lambda gd: -gd[1]["swing"])[:2]:
        add(g, "Leafs game with the most drama")
    if leafs:
        add(max(leafs, key=lambda gd: gd[0]["date"])[0], "Latest Leafs game")

    out_dir = os.path.join(ROOT, "web", "games")
    if os.path.isdir(out_dir):
        shutil.rmtree(out_dir)
    os.makedirs(out_dir)
    index = []
    for g, why in picks:
        with open(os.path.join(out_dir, f'{g["id"]}.json'), "w") as f:
            json.dump(replay_file(g), f, separators=(",", ":"))
        index.append({"id": g["id"], "label": label(g), "why": why,
                      "home": g["home"]["abbrev"], "away": g["away"]["abbrev"], "date": g["date"]})
    with open(os.path.join(out_dir, "index.json"), "w") as f:
        json.dump(index, f, indent=1)
    for name in ("wp_model.json", "report.json"):
        shutil.copy(os.path.join(ROOT, "model", name), os.path.join(ROOT, "web", name))
    print(f"exported {len(index)} games")
    for row in index:
        print(" ", row["label"], "|", row["why"])


if __name__ == "__main__":
    main()
