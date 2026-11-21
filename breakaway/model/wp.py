"""Win-probability features. The browser has an identical copy of this logic in
web/wp.js; tests/test_parity.py checks that the two agree to 1e-9.

A game state is everything known right after an event:
    sec        seconds since puck drop (>= 3600 means overtime)
    home, away current score
    situation  NHL situationCode, e.g. "1551" =
               away goalie(1) away skaters(5) home skaters(5) home goalie(1)
    elo_diff   pregame rating difference, home minus away (home ice included)
    shootout   True once the game is in a shootout
"""
from __future__ import annotations

import math

REG_SECONDS = 3600

# Regulation model features. Each is chosen to have a plain meaning:
REG_FEATURES = [
    "lead",              # goal difference (home - away)
    "lead_late",         # lead / sqrt(minutes left + 1): a lead matters more as time runs out
    "lead_very_late",    # lead / (minutes left + 1): ...and a lot more in the last minutes
    "strength",          # pregame rating gap (per 100 Elo)
    "strength_x_time",   # rating gap weighted by the share of the game still to play
    "power_play",        # skater advantage (home - away), e.g. +1 on a home power play
    "power_play_late",   # skater advantage / sqrt(minutes left + 1)
    "goalie_pulled",     # (away net empty) - (home net empty), scaled toward the end
]
OT_FEATURES = ["strength", "power_play"]


def parse_situation(code: str) -> tuple[int, int, int, int]:
    """-> (away_goalie, away_skaters, home_skaters, home_goalie); defaults to 5v5."""
    if not code or len(code) != 4 or not code.isdigit():
        return 1, 5, 5, 1
    return int(code[0]), int(code[1]), int(code[2]), int(code[3])


def reg_features(sec: float, home: int, away: int, situation: str, elo_diff: float) -> list[float]:
    mins_left = max(0.0, (REG_SECONDS - sec) / 60.0)
    lead = home - away
    ag, ask, hsk, hg = parse_situation(situation)
    pp = hsk - ask
    # a pulled goalie adds a skater; count it as the empty-net signal instead
    if hg == 0:
        pp -= 1
    if ag == 0:
        pp += 1
    pulled = (1 - ag) - (1 - hg)
    root = math.sqrt(mins_left + 1.0)
    strength = elo_diff / 100.0
    return [
        lead,
        lead / root,
        lead / (mins_left + 1.0),
        strength,
        strength * (mins_left / 60.0),
        pp,
        pp / root,
        pulled / root,
    ]


def ot_features(situation: str, elo_diff: float) -> list[float]:
    ag, ask, hsk, hg = parse_situation(situation)
    return [elo_diff / 100.0, hsk - ask]


def sigmoid(z: float) -> float:
    if z >= 0:
        return 1.0 / (1.0 + math.exp(-z))
    e = math.exp(z)
    return e / (1.0 + e)


def predict(model: dict, sec: float, home: int, away: int, situation: str,
            elo_diff: float, shootout: bool = False, final: bool = False) -> float:
    """P(home team wins)."""
    if final:
        return 1.0 if home > away else 0.0 if away > home else 0.5
    if sec < REG_SECONDS:
        x = reg_features(sec, home, away, situation, elo_diff)
        m = model["regulation"]
    elif home != away:
        return 1.0 if home > away else 0.0  # sudden death: a lead in OT ends it
    elif shootout:
        x = [elo_diff / 100.0, 0.0]
        m = model["shootout"]
    else:
        x = ot_features(situation, elo_diff)
        m = model["overtime"]
    z = m["intercept"] + sum(c * v for c, v in zip(m["coef"], x))
    return sigmoid(z)
