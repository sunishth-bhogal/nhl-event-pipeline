import { useCallback, useEffect, useRef, useState } from "react";
import type { ActivityEntry, MetricsSnapshot } from "@nhl/schema";
import { CONSUMER_API_URL, CONSUMER_WS_URL } from "./config";

const ACTIVITY_LOG_CAP = 60;

export interface GameSummary {
  gameId: string;
  playCount: number;
  latestGameClockMs: number;
}

export function useConsumerFeed() {
  const [metrics, setMetrics] = useState<MetricsSnapshot | null>(null);
  const [activity, setActivity] = useState<ActivityEntry[]>([]);
  const [connected, setConnected] = useState(false);
  const activityRef = useRef<ActivityEntry[]>([]);

  useEffect(() => {
    let cancelled = false;
    let retryTimer: ReturnType<typeof setTimeout>;

    function connect() {
      const ws = new WebSocket(CONSUMER_WS_URL);
      ws.onopen = () => !cancelled && setConnected(true);
      ws.onclose = () => {
        if (cancelled) return;
        setConnected(false);
        retryTimer = setTimeout(connect, 1500);
      };
      ws.onmessage = (evt) => {
        const msg = JSON.parse(evt.data);
        if (msg.type === "metrics") setMetrics(msg.metrics);
        if (msg.type === "activity") {
          activityRef.current = [msg.entry, ...activityRef.current].slice(0, ACTIVITY_LOG_CAP);
          setActivity(activityRef.current);
        }
      };
    }
    connect();
    return () => {
      cancelled = true;
      clearTimeout(retryTimer);
    };
  }, []);

  return { metrics, activity, connected };
}

export function useGamesList(refreshToken: unknown) {
  const [games, setGames] = useState<GameSummary[]>([]);

  useEffect(() => {
    let cancelled = false;
    fetch(`${CONSUMER_API_URL}/games`)
      .then((r) => r.json())
      .then((data) => !cancelled && setGames(data.games))
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [refreshToken]);

  return games;
}

export function usePlays(gameId: string | null, refreshToken: unknown) {
  const [plays, setPlays] = useState<any[]>([]);

  const refetch = useCallback(() => {
    if (!gameId) {
      setPlays([]);
      return;
    }
    fetch(`${CONSUMER_API_URL}/games/${gameId}/plays`)
      .then((r) => r.json())
      .then((data) => setPlays(data.plays))
      .catch(() => {});
  }, [gameId]);

  useEffect(() => {
    refetch();
  }, [refetch, refreshToken]);

  return plays;
}
