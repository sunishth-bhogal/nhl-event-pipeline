"""Second-by-second fair price for each featured game, for the market simulator.

    python model/fair_paths.py      # after export_games.py

Writes engine/data/fair/<id>.csv:
    first line   home_won,<0|1>
    then         sec,cents     fair price of "home wins" in cents (0-100, float)
The state at second s includes every event at or before s, and the model is the
same one the replay page uses.
"""
from __future__ import annotations

import json
import os
import sys

sys.path.insert(0, os.path.dirname(__file__))
from wp import predict  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def fair_path(model: dict, g: dict) -> list[tuple[int, float]]:
    plays = g["plays"]
    end = int(plays[-1][0]) if plays else 3600
    hs = as_ = 0
    sit = "1551"
    tie = 0
    period = 1
    so = final = False
    i = 0
    out = []
    for s in range(0, end + 1):
        while i < len(plays) and plays[i][0] <= s:
            sec, period, kind, owner, sit_i, h, a = plays[i][:7]
            so = g["type"] == 2 and period >= 5
            final = kind in ("END", "SOC")
            if not so and not final:
                tie = h
            hs, as_ = (g["final"]["home"], g["final"]["away"]) if final else (tie, tie) if so else (h, a)
            sit = sit_i or sit
            i += 1
        p = predict(model, s, hs, as_, sit, g["elo_diff"], so, final)
        out.append((s, round(p * 100, 3)))
    return out


def main():
    model = json.load(open(os.path.join(ROOT, "web", "wp_model.json")))
    index = json.load(open(os.path.join(ROOT, "web", "games", "index.json")))
    out_dir = os.path.join(ROOT, "engine", "data", "fair")
    os.makedirs(out_dir, exist_ok=True)
    for row in index:
        g = json.load(open(os.path.join(ROOT, "web", "games", f'{row["id"]}.json')))
        path = fair_path(model, g)
        home_won = int(g["final"]["home"] > g["final"]["away"])
        with open(os.path.join(out_dir, f'{g["id"]}.csv'), "w") as f:
            f.write(f"home_won,{home_won}\n")
            f.writelines(f"{s},{p}\n" for s, p in path)
        print(f'{g["id"]}: {len(path)} seconds, home_won={home_won}')


if __name__ == "__main__":
    main()
