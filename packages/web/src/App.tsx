import { useEffect, useState } from "react";
import { useControlState } from "./lib/useControlState";
import { useConsumerFeed, useGamesList, usePlays } from "./lib/useConsumerFeed";
import { FailureLabPanel } from "./components/FailureLabPanel";
import { ReplayControls } from "./components/ReplayControls";
import { MetricsBar } from "./components/MetricsBar";
import { ActivityTicker } from "./components/ActivityTicker";
import { PlayByPlay } from "./components/PlayByPlay";
import { GameSelector } from "./components/GameSelector";

function StatusDot({ ok }: { ok: boolean }) {
  return <span className={`inline-block w-2 h-2 rounded-full ${ok ? "bg-emerald-500" : "bg-rose-500"}`} />;
}

export default function App() {
  const { state: control, connected: controlConnected, patchFailureLab, patchReplay } = useControlState();
  const { metrics, activity, connected: consumerConnected } = useConsumerFeed();
  const games = useGamesList(activity.length);
  const [selectedGame, setSelectedGame] = useState<string | null>(null);
  const plays = usePlays(selectedGame, activity.length);

  useEffect(() => {
    if (!selectedGame && games.length > 0) setSelectedGame(games[0].gameId);
  }, [games, selectedGame]);

  return (
    <div className="min-h-screen max-w-[1600px] mx-auto px-6 py-6 space-y-6">
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold text-slate-100">NHL Event Pipeline — Failure Lab</h1>
          <p className="text-sm text-slate-500">
            Kafka-backed streaming pipeline with idempotent dedup, event-time watermarking, and live failure
            injection.
          </p>
        </div>
        <div className="flex items-center gap-4 text-xs text-slate-400">
          <span className="flex items-center gap-1.5">
            <StatusDot ok={controlConnected} /> control-api
          </span>
          <span className="flex items-center gap-1.5">
            <StatusDot ok={consumerConnected} /> consumer
          </span>
        </div>
      </header>

      <MetricsBar metrics={metrics} />

      <div className="grid grid-cols-1 lg:grid-cols-[320px_1fr_360px] gap-6">
        <aside className="space-y-6">
          <section className="bg-slate-900/40 border border-slate-800 rounded-lg p-4">
            <h2 className="text-sm font-semibold text-slate-200 mb-3">Replay control</h2>
            <ReplayControls state={control?.replay} onChange={patchReplay} />
          </section>
          <section className="bg-slate-900/40 border border-slate-800 rounded-lg p-4">
            <h2 className="text-sm font-semibold text-slate-200 mb-3">Failure Lab</h2>
            <FailureLabPanel state={control?.failureLab} onChange={patchFailureLab} />
          </section>
        </aside>

        <main className="bg-slate-900/40 border border-slate-800 rounded-lg p-4 flex flex-col min-h-[500px]">
          <div className="flex items-center justify-between mb-3">
            <h2 className="text-sm font-semibold text-slate-200">Play-by-play (finalized, event-time order)</h2>
            <GameSelector games={games} selected={selectedGame} onSelect={setSelectedGame} />
          </div>
          <div className="flex-1 min-h-0">
            <PlayByPlay plays={plays} />
          </div>
        </main>

        <section className="bg-slate-900/40 border border-slate-800 rounded-lg p-4 flex flex-col min-h-[500px]">
          <h2 className="text-sm font-semibold text-slate-200 mb-3">Live activity</h2>
          <div className="flex-1 min-h-0">
            <ActivityTicker entries={activity} />
          </div>
        </section>
      </div>
    </div>
  );
}
