# Breakaway

A live hockey exchange, built in stages.

**Step 1 (done): win probability you can watch.** Replay any NHL game as
if it were live and watch each team's chance of winning move with every goal,
penalty and pulled goalie.

Later steps add an order book and matching engine, a market-making bot that
quotes around the model, play-money accounts, and live games.

## Layout

```
pipeline/   fetch_games.py downloads play-by-play (runs in GitHub Actions)
            SEASONS lists which seasons to fetch; editing it triggers the download
data/       compact games, one gzipped JSON-lines file per season
model/      wp.py (features + prediction), train.py (training + evaluation)
web/        the replay player (static HTML/JS, no build step)
tests/      python3 tests/test_*.py
```

Data comes from the NHL's public (undocumented) web API, fetched gently.

## Run it

```
python3 model/train.py          # train + evaluate -> model/wp_model.json, model/report.json
python3 model/export_games.py   # featured replays -> web/games/
cd web && python3 -m http.server 8000   # open http://localhost:8000
python3 tests/test_fetch.py && python3 tests/test_parity.py
```

## Model results (held-out season)

Trained on 5,599 games (2021-22 to 2024-25), tested on all
1,394 games of 2025-26, which it never saw. Every game
state is weighted by how many seconds of game time it lasted.

| | Brier | Log loss |
|---|---:|---:|
| Breakaway model (logistic regression) | 0.1766 | 0.5160 |
| Gradient boosting on the raw state (benchmark) | 0.1787 | 0.5230 |
| Score and clock only | 0.1760 | 0.5153 |
| Pregame ratings only | 0.2494 | 0.6921 |

How often the model's favourite went on to win: 53% at puck drop,
64% after the 1st, 76% after the 2nd,
83% with 5 minutes left.

What this says, honestly:

* The simple model matches a flexible gradient-boosting model, so it ships: it
  runs in the browser and every coefficient has a plain meaning.
* Pregame team strength added almost nothing in 2025-26. That season was
  unusually even: the team with the better record so far won only 54.6% of
  games (62% in 2021-22), and home teams won 52.2%. Elo settings were chosen on
  2024-25, not on the test season.
* Calibration is good at the extremes; near 50/50 the model slightly
  overrates home ice for this season.

Next for the model: shot-quality (expected goals) so pressure moves the line
before a goal, and a better pregame rating.
