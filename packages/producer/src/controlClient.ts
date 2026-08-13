import WebSocket from "ws";
import { EventEmitter } from "node:events";
import { type ControlState, defaultControlState } from "@nhl/schema";

const CONTROL_WS_URL = process.env.CONTROL_WS_URL ?? "ws://localhost:4000/ws";

/** Thin client that keeps a live copy of the control plane's state, pushed
 * over WebSocket. The producer just reads `.state` synchronously wherever it
 * needs to decide whether to duplicate/delay/drop/reorder the next publish. */
class ControlClient extends EventEmitter {
  state: ControlState = defaultControlState();

  connect() {
    const attempt = () => {
      const ws = new WebSocket(CONTROL_WS_URL);
      ws.on("open", () => console.log("[producer] connected to control-api"));
      ws.on("message", (raw) => {
        try {
          const msg = JSON.parse(raw.toString());
          if (msg.type === "state") {
            const prev = this.state;
            this.state = msg.state;
            this.emit("state", this.state, prev);
          }
        } catch (err) {
          console.error("[producer] bad control message", err);
        }
      });
      ws.on("close", () => {
        console.warn("[producer] control-api connection lost, retrying in 2s");
        setTimeout(attempt, 2000);
      });
      ws.on("error", () => ws.close());
    };
    attempt();
  }
}

export const controlClient = new ControlClient();
