"""Train and evaluate the win-probability model.

    python model/train.py            # uses data/games-*.jsonl.gz

Writes:
    model/wp_model.json   coefficients + team ratings, loaded by the browser
    model/report.json     held-out evaluation (Brier, log loss, calibration)

Honest-evaluation rules:
  * Team ratings (Elo) for a game only use games played *before* it.
  * The test season is never seen during training.
"""
from __future__ import annotations

import glob
import gzip
import json
import math
import os
import sys
from collections import defaultdict

import numpy as np
from sklearn.ensemble import HistGradientBoostingClassifier
from sklearn.linear_model import LogisticRegression

sys.path.insert(0, os.path.dirname(__file__))
from wp import OT_FEATURES, REG_FEATURES, REG_SECONDS, ot_features, parse_situation, reg_features  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# ---------------------------------------------------------------- Elo ratings
# Chosen on the validation season (2024-25, the one before the test season) by
# pregame log loss over a small grid: K in {6,12,20,30}, home ice in {15,30,45},
# carry-over in {0.5,0.7}, with/without a margin-of-victory multiplier.
# The test season played no part in the choice.
ELO_K = 6.0
ELO_HOME = 45.0          # home-ice advantage in Elo points
ELO_CARRY = 0.7          # share of a rating carried into the next season
ELO_USE_MARGIN = False


def load_games() -> list[dict]:
    games = []
    for path in sorted(glob.glob(os.path.join(ROOT, "data", "games-*.jsonl.gz"))):
        with gzip.open(path, "rt", encoding="utf-8") as f:
            games.extend(json.loads(line) for line in f)
    games.sort(key=lambda g: (g["date"], g["id"]))
    return games


def attach_elo(games: list[dict]) -> dict[int, float]:
    """Sets g["elo_diff"] (home - away + home ice) from games strictly before g."""
    rating: dict[int, float] = defaultdict(lambda: 1500.0)
    season = None
    for g in games:
        if g["season"] != season:
            season = g["season"]
            for t in list(rating):
                rating[t] = 1500.0 + ELO_CARRY * (rating[t] - 1500.0)
        h, a = g["home"]["id"], g["away"]["id"]
        g["elo_diff"] = rating[h] - rating[a] + ELO_HOME
        expected = 1.0 / (1.0 + 10 ** (-g["elo_diff"] / 400.0))
        hs, as_ = g["final"]["home"], g["final"]["away"]
        won = 1.0 if hs > as_ else 0.0
        mult = 1.0
        if ELO_USE_MARGIN:
            mult = math.log(abs(hs - as_) + 1.0) + (0.5 if g["final"]["last"] == "REG" else 0.0)
        delta = ELO_K * mult * (won - expected)
        rating[h] += delta
        rating[a] -= delta
    return dict(rating)


# ---------------------------------------------------------------- states
def states(g: dict):
    """Yield (kind, features, weight, sec) for the game state after each event.
    Each state is weighted by how long the game stayed in it, so the model is
    calibrated over game time rather than over event counts."""
    plays = [p for p in g["plays"] if p[2] not in ("END", "SOC")]
    start = [0, 1, "PS", -1, "1551", 0, 0, None, None, ""]
    seq = [start] + plays
    for i, p in enumerate(seq):
        sec, period, kind, owner, sit, hs, as_ = p[:7]
        nxt = seq[i + 1][0] if i + 1 < len(seq) else sec
        weight = max(nxt - sec, 1)
        if sec < REG_SECONDS:
            yield "reg", reg_features(sec, hs, as_, sit, g["elo_diff"]), weight, sec
        elif hs == as_:
            if g["type"] == 2 and period >= 5:
                continue  # regular-season shootout: handled by the shootout model
            yield "ot", ot_features(sit, g["elo_diff"]), weight, sec


def build(games: list[dict]):
    reg_x, reg_y, reg_w, reg_t = [], [], [], []
    ot_x, ot_y, ot_w = [], [], []
    so_x, so_y = [], []
    for g in games:
        y = 1 if g["final"]["home"] > g["final"]["away"] else 0
        for kind, x, w, sec in states(g):
            if kind == "reg":
                reg_x.append(x); reg_y.append(y); reg_w.append(w); reg_t.append(sec)
            else:
                ot_x.append(x); ot_y.append(y); ot_w.append(w)
        if g["final"]["last"] == "SO":
            so_x.append([g["elo_diff"] / 100.0, 0.0]); so_y.append(y)
    arr = lambda v: np.asarray(v, dtype=float)
    return (arr(reg_x), arr(reg_y), arr(reg_w), arr(reg_t),
            arr(ot_x), arr(ot_y), arr(ot_w), arr(so_x), arr(so_y))


# ---------------------------------------------------------------- metrics
def brier(p, y, w):
    return float(np.average((p - y) ** 2, weights=w))


def logloss(p, y, w):
    p = np.clip(p, 1e-6, 1 - 1e-6)
    return float(np.average(-(y * np.log(p) + (1 - y) * np.log(1 - p)), weights=w))


def calibration(p, y, w, bins=10):
    out = []
    edges = np.linspace(0, 1, bins + 1)
    for lo, hi in zip(edges[:-1], edges[1:]):
        m = (p >= lo) & (p < hi if hi < 1 else p <= hi)
        if w[m].sum() == 0:
            continue
        out.append({
            "bin": f"{lo:.1f}-{hi:.1f}",
            "predicted": round(float(np.average(p[m], weights=w[m])), 4),
            "actual": round(float(np.average(y[m], weights=w[m])), 4),
            "share_of_game_time": round(float(w[m].sum() / w.sum()), 4),
        })
    return out


def fit_logit(x, y, w):
    if len(set(y.tolist())) < 2:
        raise SystemExit(f"not enough data to fit a model ({len(y)} rows, one outcome)")
    m = LogisticRegression(C=10.0, max_iter=2000)
    m.fit(x, y, sample_weight=w)
    return m


def as_json(m) -> dict:
    return {"intercept": float(m.intercept_[0]), "coef": [float(c) for c in m.coef_[0]]}


def main():
    games = load_games()
    if not games:
        sys.exit("no data: run the fetch first (see pipeline/SEASONS)")
    ratings = attach_elo(games)
    seasons = sorted({g["season"] for g in games})
    # test on the most recent *complete* season; the current season is only partial
    complete = [s for s in seasons if sum(1 for g in games if g["season"] == s and g["type"] == 2) >= 1300]
    test_season = complete[-1]
    train_games = [g for g in games if g["season"] < test_season]
    test_games = [g for g in games if g["season"] == test_season]
    print(f"{len(games)} games; train seasons {seasons[0]}..{test_season - 10001} "
          f"({len(train_games)}), test {test_season} ({len(test_games)})")

    Xr, yr, wr, tr, Xo, yo, wo, Xs, ys = build(train_games)
    reg = fit_logit(Xr, yr, wr)
    ot = fit_logit(Xo, yo, wo)
    so = fit_logit(Xs, ys, np.ones(len(ys)))

    # benchmark: a flexible model on the raw state, to see what the simple one gives up
    def raw(X, T):
        mins_left = (REG_SECONDS - T) / 60.0
        return np.column_stack([X[:, 0], mins_left, X[:, 3], X[:, 5], X[:, 7]])
    gbm = HistGradientBoostingClassifier(max_iter=300, learning_rate=0.05, max_leaf_nodes=31)
    gbm.fit(raw(Xr, tr), yr, sample_weight=wr)

    tXr, tyr, twr, ttr, *_ = build(test_games)
    p_model = reg.predict_proba(tXr)[:, 1]
    p_gbm = gbm.predict_proba(raw(tXr, ttr))[:, 1]
    # baselines: pregame rating only; and "the leader wins" from lead alone
    p_pre = 1.0 / (1.0 + 10 ** (-(tXr[:, 3] * 100.0) / 400.0))
    lead_only = fit_logit(Xr[:, [0, 1, 2]], yr, wr)
    p_lead = lead_only.predict_proba(tXr[:, [0, 1, 2]])[:, 1]

    def scores(p):
        return {"brier": round(brier(p, tyr, twr), 5), "log_loss": round(logloss(p, tyr, twr), 5)}

    checkpoints = {}
    for name, sec in (("puck drop", 0), ("after 1st", 1200), ("after 2nd", 2400), ("5 min left", 3300), ("1 min left", 3540)):
        # last state at or before the checkpoint in each game
        ps, ys_ = [], []
        for g in test_games:
            last = None
            for kind, x, w, s in states(g):
                if kind == "reg" and s <= sec:
                    last = x
            if last is not None:
                ps.append(float(reg.predict_proba([last])[0, 1]))
                ys_.append(1 if g["final"]["home"] > g["final"]["away"] else 0)
        ps, ys_ = np.asarray(ps), np.asarray(ys_)
        checkpoints[name] = {
            "games": int(len(ps)),
            "brier": round(float(np.mean((ps - ys_) ** 2)), 4),
            "favourite_won": round(float(np.mean((ps > 0.5) == (ys_ == 1))), 4),
        }

    report = {
        "test_season": test_season,
        "train_games": len(train_games),
        "test_games": len(test_games),
        "test_states_weighting": "each game state weighted by seconds of game time it lasted",
        "model": scores(p_model),
        "benchmark_gradient_boosting": scores(p_gbm),
        "baseline_pregame_rating_only": scores(p_pre),
        "baseline_lead_and_time_only": scores(p_lead),
        "calibration": calibration(p_model, tyr, twr),
        "checkpoints": checkpoints,
        "overtime_states_train": int(len(yo)),
        "shootouts_train": int(len(ys)),
    }

    names = {g["home"]["id"]: g["home"]["abbrev"] for g in games}
    names.update({g["away"]["id"]: g["away"]["abbrev"] for g in games})
    model = {
        "version": 1,
        "trained_on_seasons": sorted({g["season"] for g in train_games}),
        "regulation": {**as_json(reg), "features": REG_FEATURES},
        "overtime": {**as_json(ot), "features": OT_FEATURES},
        "shootout": {**as_json(so), "features": ["strength", "unused"]},
        "elo": {"home_ice": ELO_HOME, "k": ELO_K, "season_carry": ELO_CARRY,
                "ratings": {names[t]: round(r, 1) for t, r in sorted(ratings.items(), key=lambda kv: -kv[1])}},
    }
    with open(os.path.join(ROOT, "model", "wp_model.json"), "w") as f:
        json.dump(model, f, indent=2)
    with open(os.path.join(ROOT, "model", "report.json"), "w") as f:
        json.dump(report, f, indent=2)
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
