import type { MetricsSnapshot } from "@nhl/schema";

interface Props {
  metrics: MetricsSnapshot | null;
}

function Tile({ label, value, accent }: { label: string; value: string; accent?: string }) {
  return (
    <div className="bg-slate-900 border border-slate-800 rounded-lg px-4 py-3">
      <div className="text-xs uppercase tracking-wide text-slate-500 mb-1">{label}</div>
      <div className={`text-2xl font-mono font-semibold ${accent ?? "text-slate-100"}`}>{value}</div>
    </div>
  );
}

export function MetricsBar({ metrics }: Props) {
  const m = metrics;
  return (
    <div className="grid grid-cols-2 sm:grid-cols-4 lg:grid-cols-8 gap-3">
      <Tile label="Ingested" value={String(m?.totalIngested ?? 0)} />
      <Tile label="Events/sec" value={(m?.eventsPerSec ?? 0).toFixed(1)} />
      <Tile label="Duplicates suppressed" value={String(m?.duplicatesDropped ?? 0)} accent="text-amber-400" />
      <Tile label="Late recovered" value={String(m?.lateRecovered ?? 0)} accent="text-sky-400" />
      <Tile label="Late discarded" value={String(m?.lateDiscarded ?? 0)} accent="text-rose-400" />
      <Tile label="Gaps detected" value={String(m?.gapsDetected ?? 0)} accent="text-rose-400" />
      <Tile label="Latency p50/p95" value={`${Math.round(m?.latencyP50Ms ?? 0)}/${Math.round(m?.latencyP95Ms ?? 0)}ms`} />
      <Tile label="Watermark lag" value={`${Math.round(m?.watermarkLagMs ?? 0)}ms`} />
    </div>
  );
}
