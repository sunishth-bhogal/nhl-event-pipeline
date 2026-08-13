import http from "node:http";
import express from "express";
import cors from "cors";
import { WebSocketServer } from "ws";
import type { ActivityEntry } from "@nhl/schema";
import { db, getPlays } from "./db.js";
import { metrics } from "./metrics.js";
import { activityBus } from "./watermark.js";

const PORT = Number(process.env.CONSUMER_PORT ?? 4001);

const app = express();
app.use(cors());

app.get("/metrics", (_req, res) => {
  res.json(metrics.snapshot());
});

app.get("/games", (_req, res) => {
  const rows = db
    .prepare(
      "SELECT game_id as gameId, COUNT(*) as playCount, MAX(game_clock_ms) as latestGameClockMs FROM plays GROUP BY game_id ORDER BY MAX(processed_at) DESC",
    )
    .all();
  res.json({ games: rows });
});

app.get("/games/:gameId/plays", (req, res) => {
  res.json({ plays: getPlays(req.params.gameId) });
});

const server = http.createServer(app);
const wss = new WebSocketServer({ server, path: "/ws" });

function broadcast(payload: unknown) {
  const msg = JSON.stringify(payload);
  for (const client of wss.clients) {
    if (client.readyState === client.OPEN) client.send(msg);
  }
}

activityBus.on("activity", (entry: ActivityEntry) => {
  broadcast({ type: "activity", entry });
});

setInterval(() => {
  broadcast({ type: "metrics", metrics: metrics.snapshot() });
}, 1000);

export function startServer() {
  server.listen(PORT, () => {
    console.log(`[consumer] observability API on http://localhost:${PORT}`);
  });
}
