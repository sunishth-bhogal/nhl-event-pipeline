import type { ActivityEntry } from "@nhl/schema";

const KIND_STYLES: Record<string, { label: string; className: string }> = {
  ingested: { label: "OK", className: "bg-slate-800 text-slate-300" },
  "duplicate-suppressed": { label: "DUPLICATE", className: "bg-amber-500/20 text-amber-400" },
  "late-recovered": { label: "LATE (recovered)", className: "bg-sky-500/20 text-sky-400" },
  "late-discarded": { label: "LATE (discarded)", className: "bg-rose-500/20 text-rose-400" },
  "gap-detected": { label: "GAP", className: "bg-rose-500/20 text-rose-400" },
};

export function ActivityTicker({ entries }: { entries: ActivityEntry[] }) {
  return (
    <div className="h-full overflow-y-auto space-y-1.5 pr-1">
      {entries.length === 0 && (
        <div className="text-slate-500 text-sm py-8 text-center">
          No activity yet — start a replay to see the pipeline work.
        </div>
      )}
      {entries.map((e) => {
        const style = KIND_STYLES[e.kind] ?? KIND_STYLES.ingested;
        return (
          <div
            key={e.id}
            className="flex items-start gap-2 text-sm bg-slate-900/60 border border-slate-800 rounded-md px-2.5 py-1.5"
          >
            <span className={`shrink-0 text-[10px] font-mono font-bold px-1.5 py-0.5 rounded ${style.className}`}>
              {style.label}
            </span>
            <span className="text-slate-300 truncate">{e.description}</span>
          </div>
        );
      })}
    </div>
  );
}
