import { useEffect, useState } from "react";
import type { ReplayState } from "@nhl/schema";
import { CONTROL_API_URL } from "../lib/config";

interface Props {
  state: ReplayState | undefined;
  onChange: (patch: Partial<ReplayState>) => void;
}

const SPEEDS = [1, 2, 5, 10, 25];

export function ReplayControls({ state, onChange }: Props) {
  const [fixtures, setFixtures] = useState<string[]>([]);
  const [liveGameId, setLiveGameId] = useState("");

  useEffect(() => {
    fetch(`${CONTROL_API_URL}/fixtures`)
      .then((r) => r.json())
      .then((d) => setFixtures(d.fixtures))
      .catch(() => {});
  }, []);

  if (!state) return <div className="text-slate-500 text-sm">Connecting to control plane…</div>;

  const isReplaying = state.mode === "replay";
  const isLive = state.mode === "live";

  return (
    <div className="space-y-4">
      <div>
        <div className="text-sm text-slate-300 mb-1">Fixture (recorded game)</div>
        <select
          className="w-full bg-slate-800 border border-slate-700 rounded-md px-2 py-1.5 text-sm"
          value={state.fixture ?? ""}
          onChange={(e) => onChange({ fixture: e.target.value || null })}
        >
          <option value="">Select a fixture…</option>
          {fixtures.map((f) => (
            <option key={f} value={f}>
              {f}
            </option>
          ))}
        </select>
      </div>

      <div>
        <div className="text-sm text-slate-300 mb-1">Playback speed</div>
        <div className="flex gap-2">
          {SPEEDS.map((s) => (
            <button
              key={s}
              onClick={() => onChange({ speed: s })}
              className={`px-3 py-1 rounded-md text-sm font-mono transition-colors ${
                state.speed === s
                  ? "bg-emerald-600 text-white"
                  : "bg-slate-800 text-slate-300 hover:bg-slate-700"
              }`}
            >
              {s}x
            </button>
          ))}
        </div>
      </div>

      <button
        disabled={!state.fixture}
        onClick={() => onChange({ mode: isReplaying ? "idle" : "replay" })}
        className="w-full py-2 rounded-md bg-emerald-600 hover:bg-emerald-500 disabled:bg-slate-800 disabled:text-slate-500 text-white text-sm font-medium transition-colors"
      >
        {isReplaying ? "Stop replay" : "Start replay"}
      </button>

      <div className="pt-3 border-t border-slate-800 space-y-2">
        <div className="text-sm text-slate-300">Live NHL game (in progress)</div>
        <input
          value={liveGameId}
          onChange={(e) => setLiveGameId(e.target.value)}
          placeholder="e.g. 2024021296"
          className="w-full bg-slate-800 border border-slate-700 rounded-md px-2 py-1.5 text-sm font-mono"
        />
        <button
          disabled={!liveGameId}
          onClick={() => onChange({ mode: isLive ? "idle" : "live", gameId: liveGameId || null })}
          className="w-full py-2 rounded-md bg-sky-700 hover:bg-sky-600 disabled:bg-slate-800 disabled:text-slate-500 text-white text-sm font-medium transition-colors"
        >
          {isLive ? "Stop live polling" : "Poll live game"}
        </button>
      </div>

      <div className="text-xs text-slate-500">
        Mode: <span className="font-mono text-slate-300">{state.mode}</span>
      </div>
    </div>
  );
}
