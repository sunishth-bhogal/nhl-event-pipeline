"""Simulated seasons in the compact format, for exercising the pipeline
before (or without) real data. Teams have hidden strengths; goals arrive as a
Poisson process boosted on the power play."""
import gzip, json, os, random, sys

def simulate(season, rng, teams=16, games_per=82):
    strength = {t: rng.gauss(0, 0.25) for t in range(1, teams + 1)}
    out, gid = [], 0
    for day in range(teams * games_per // 2):
        h, a = rng.sample(range(1, teams + 1), 2)
        gid += 1
        rate_h = 3.0 / 3600 * (1.05 + strength[h] - strength[a])
        rate_a = 3.0 / 3600 * (0.95 + strength[a] - strength[h])
        hs = as_ = 0; plays = [[0, 1, "PS", -1, "1551", 0, 0, None, None, ""]]
        pp_until, pp_side = -1, None
        for sec in range(1, 3601):
            sit = "1551"
            if sec < pp_until:
                sit = "1451" if pp_side == 1 else "1541"
            elif rng.random() < 1 / 400:
                pp_until, pp_side = sec + 120, rng.choice([0, 1])
                plays.append([sec, (sec - 1) // 1200 + 1, "P", 1 - pp_side, sit, hs, as_, None, None, "x"])
            bh = 1.8 if sit == "1451" else 0.6 if sit == "1541" else 1
            ba = 1.8 if sit == "1541" else 0.6 if sit == "1451" else 1
            if rng.random() < rate_h * bh:
                hs += 1; plays.append([sec, (sec - 1) // 1200 + 1, "G", 1, sit, hs, as_, 0, 0, "g"]); pp_until = -1
            elif rng.random() < rate_a * ba:
                as_ += 1; plays.append([sec, (sec - 1) // 1200 + 1, "G", 0, sit, hs, as_, 0, 0, "g"]); pp_until = -1
        last = "REG"
        if hs == as_:
            last = "OT"
            plays.append([3600, 4, "PS", -1, "1331", hs, as_, None, None, ""])
            for sec in range(3601, 3901):
                if rng.random() < 1 / 400:
                    if rng.random() < 0.5 + strength[h] - strength[a]: hs += 1
                    else: as_ += 1
                    plays.append([sec, 4, "G", int(hs > as_), "1331", hs, as_, 0, 0, "g"]); break
            if hs == as_:
                last = "SO"
                if rng.random() < 0.5: hs += 1
                else: as_ += 1
        plays.append([max(3600, plays[-1][0]), 3, "END", -1, "1551", hs, as_, None, None, ""])
        out.append({"id": season * 10000 + gid, "season": season, "type": 2,
                    "date": f"{season // 10000}-{10 + day * 6 // (teams * games_per // 2):02d}-{1 + day % 28:02d}",
                    "home": {"id": h, "abbrev": f"T{h:02d}", "name": f"Team {h}"},
                    "away": {"id": a, "abbrev": f"T{a:02d}", "name": f"Team {a}"},
                    "final": {"home": hs, "away": as_, "last": last}, "plays": plays})
    return out

if __name__ == "__main__":
    dest = sys.argv[1]
    os.makedirs(dest, exist_ok=True)
    rng = random.Random(7)
    for season in (20212022, 20222023, 20232024):
        games = simulate(season, rng, games_per=82 * 2)  # 1312 games -> counts as a full season
        with gzip.open(f"{dest}/games-{season}.jsonl.gz", "wt") as f:
            for g in games: f.write(json.dumps(g) + "\n")
    print("ok")
