interface PlayRow {
  event_id: string;
  period: number;
  period_time_remaining: string;
  event_type: string;
  team_abbrev: string | null;
  description: string;
  was_late: number;
  late_by_ms: number;
  source: string;
}

function formatClock(period: number, remaining: string) {
  const label = period <= 3 ? `P${period}` : "OT";
  return `${label} ${remaining}`;
}

export function PlayByPlay({ plays }: { plays: PlayRow[] }) {
  if (plays.length === 0) {
    return (
      <div className="text-slate-500 text-sm py-8 text-center">
        No plays finalized yet for this game.
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto space-y-1">
      {plays.map((p) => (
        <div
          key={p.event_id}
          className="flex items-center gap-3 text-sm px-2.5 py-1.5 rounded-md hover:bg-slate-900/60"
        >
          <span className="w-20 shrink-0 font-mono text-xs text-slate-500">
            {formatClock(p.period, p.period_time_remaining)}
          </span>
          {p.team_abbrev && (
            <span className="w-10 shrink-0 font-mono text-xs text-slate-400">{p.team_abbrev}</span>
          )}
          <span className="flex-1 text-slate-200 truncate">{p.description}</span>
          {p.was_late === 1 && (
            <span className="shrink-0 text-[10px] font-mono font-bold px-1.5 py-0.5 rounded bg-sky-500/20 text-sky-400">
              +{p.late_by_ms}ms late
            </span>
          )}
        </div>
      ))}
    </div>
  );
}
