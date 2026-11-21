# Breakaway

A live hockey exchange, built in stages.

**Step 1 (in progress): win probability you can watch.** Replay any NHL game as
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
