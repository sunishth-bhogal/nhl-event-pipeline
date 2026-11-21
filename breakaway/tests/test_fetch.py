"""Checks the compact converter against the real API's structure (sample shaped
like gamecenter/2024030416/play-by-play)."""
import sys, pathlib
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "pipeline"))
from fetch_games import compact, game_seconds, season_ids

def play(kind, period, t, sit="1551", ptype="REG", **d):
    return {"typeDescKey": kind, "periodDescriptor": {"number": period, "periodType": ptype},
            "timeInPeriod": t, "situationCode": sit, "details": d}

RAW = {
    "id": 2024030416, "season": 20242025, "gameType": 3, "gameDate": "2025-06-17",
    "gameState": "OFF", "gameOutcome": {"lastPeriodType": "REG"},
    "homeTeam": {"id": 13, "abbrev": "FLA", "score": 5, "placeName": {"default": "Florida"}, "commonName": {"default": "Panthers"}},
    "awayTeam": {"id": 22, "abbrev": "EDM", "score": 1, "placeName": {"default": "Edmonton"}, "commonName": {"default": "Oilers"}},
    "rosterSpots": [
        {"playerId": 8477933, "teamId": 13, "firstName": {"default": "Sam"}, "lastName": {"default": "Reinhart"}},
        {"playerId": 8475169, "teamId": 22, "firstName": {"default": "Evander"}, "lastName": {"default": "Kane"}},
    ],
    "plays": [
        play("period-start", 1, "00:00"),
        play("stoppage", 1, "00:10"),
        play("shot-on-goal", 1, "01:08", eventOwnerTeamId=22, xCoord=56, yCoord=-22),
        play("goal", 1, "04:36", eventOwnerTeamId=13, homeScore=1, awayScore=0, xCoord=-72, yCoord=-6, scoringPlayerId=8477933),
        play("penalty", 1, "09:45", eventOwnerTeamId=22, committedByPlayerId=8475169, descKey="slashing", duration=2),
        play("hit", 1, "09:49", sit="1451", eventOwnerTeamId=13),
        play("goal", 4, "02:10", ptype="OT", eventOwnerTeamId=22, homeScore=1, awayScore=1),
    ],
}

def test_compact():
    g = compact(RAW)
    assert g["home"] == {"id": 13, "abbrev": "FLA", "name": "Florida Panthers"}
    assert g["final"] == {"home": 5, "away": 1, "last": "REG"}
    kinds = [p[2] for p in g["plays"]]
    assert kinds == ["PS", "S", "G", "P", "H", "G"], kinds  # stoppage dropped
    shot, goal, pen, hit, otgoal = g["plays"][1:]
    assert shot[:4] == [68, 1, "S", 0] and shot[5:7] == [0, 0]
    assert goal[:4] == [276, 1, "G", 1] and goal[5:7] == [1, 0] and goal[9] == "S. Reinhart"
    assert pen[5:7] == [1, 0], "score carried forward"
    assert pen[9] == "E. Kane · slashing (2 min)"
    assert hit[4] == "1451"
    assert otgoal[0] == 3600 + 130  # playoff OT 1

def test_seconds():
    assert game_seconds(3, "REG", "20:00", 2) == 3600
    assert game_seconds(4, "OT", "05:00", 2) == 3900
    assert game_seconds(5, "OT", "01:00", 3) == 3600 + 1200 + 60  # 2nd playoff OT
    assert game_seconds(5, "SO", "00:00", 2) == 3900

def test_ids():
    ids = season_ids(20242025)
    assert ids[0] == 2024020001 and 2024021312 in ids and 2024030416 in ids and 2024030111 in ids

def test_unfinished_skipped():
    assert compact({**RAW, "gameState": "LIVE"}) is None

if __name__ == "__main__":
    for n, f in list(globals().items()):
        if n.startswith("test_"): f(); print("ok", n)
