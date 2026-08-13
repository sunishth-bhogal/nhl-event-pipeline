import { readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import http from "node:http";
import express from "express";
import cors from "cors";
import { WebSocketServer, type WebSocket } from "ws";
import {
  FailureLabStateSchema,
  ReplayStateSchema,
} from "@nhl/schema";
import { controlStore } from "./state.js";

const PORT = Number(process.env.CONTROL_API_PORT ?? 4000);
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPLAYS_DIR = path.resolve(__dirname, "../../../data/replays");

const app = express();
app.use(cors());
app.use(express.json());

app.get("/state", (_req, res) => {
  res.json(controlStore.get());
});

app.patch("/state/failure-lab", (req, res) => {
  const patch = FailureLabStateSchema.partial().parse(req.body);
  res.json(controlStore.patchFailureLab(patch));
});

app.patch("/state/replay", (req, res) => {
  const patch = ReplayStateSchema.partial().parse(req.body);
  res.json(controlStore.patchReplay(patch));
});

app.get("/fixtures", (_req, res) => {
  try {
    const files = readdirSync(REPLAYS_DIR).filter((f) => f.endsWith(".ndjson"));
    res.json({ fixtures: files });
  } catch {
    res.json({ fixtures: [] });
  }
});

const server = http.createServer(app);
const wss = new WebSocketServer({ server, path: "/ws" });

function broadcastState() {
  const payload = JSON.stringify({ type: "state", state: controlStore.get() });
  for (const client of wss.clients) {
    if (client.readyState === client.OPEN) client.send(payload);
  }
}

controlStore.on("change", broadcastState);

wss.on("connection", (socket: WebSocket) => {
  socket.send(JSON.stringify({ type: "state", state: controlStore.get() }));
});

server.listen(PORT, () => {
  console.log(`[control-api] listening on http://localhost:${PORT}`);
});
