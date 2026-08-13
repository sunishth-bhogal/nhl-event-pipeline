import { useEffect, useRef, useState } from "react";
import type { ControlState, FailureLabState, ReplayState } from "@nhl/schema";
import { CONTROL_API_URL, CONTROL_WS_URL } from "./config";

export function useControlState() {
  const [state, setState] = useState<ControlState | null>(null);
  const [connected, setConnected] = useState(false);
  const wsRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    let cancelled = false;
    let retryTimer: ReturnType<typeof setTimeout>;

    function connect() {
      const ws = new WebSocket(CONTROL_WS_URL);
      wsRef.current = ws;
      ws.onopen = () => !cancelled && setConnected(true);
      ws.onclose = () => {
        if (cancelled) return;
        setConnected(false);
        retryTimer = setTimeout(connect, 1500);
      };
      ws.onmessage = (evt) => {
        const msg = JSON.parse(evt.data);
        if (msg.type === "state") setState(msg.state);
      };
    }
    connect();
    return () => {
      cancelled = true;
      clearTimeout(retryTimer);
      wsRef.current?.close();
    };
  }, []);

  async function patchFailureLab(patch: Partial<FailureLabState>) {
    await fetch(`${CONTROL_API_URL}/state/failure-lab`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(patch),
    });
  }

  async function patchReplay(patch: Partial<ReplayState>) {
    await fetch(`${CONTROL_API_URL}/state/replay`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(patch),
    });
  }

  return { state, connected, patchFailureLab, patchReplay };
}
