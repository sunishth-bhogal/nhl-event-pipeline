import type { FailureLabState } from "@nhl/schema";

interface Props {
  state: FailureLabState | undefined;
  onChange: (patch: Partial<FailureLabState>) => void;
}

function Slider({
  label,
  value,
  onChange,
  min = 0,
  max = 1,
  step = 0.01,
  format,
}: {
  label: string;
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  step?: number;
  format?: (v: number) => string;
}) {
  return (
    <label className="block">
      <div className="flex items-center justify-between text-sm mb-1">
        <span className="text-slate-300">{label}</span>
        <span className="font-mono text-emerald-400">{format ? format(value) : value}</span>
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="w-full accent-emerald-500"
      />
    </label>
  );
}

export function FailureLabPanel({ state, onChange }: Props) {
  if (!state) return <div className="text-slate-500 text-sm">Connecting to control plane…</div>;

  const pct = (v: number) => `${Math.round(v * 100)}%`;

  return (
    <div className="space-y-4">
      <Slider
        label="Duplicate rate"
        value={state.duplicateRate}
        onChange={(v) => onChange({ duplicateRate: v })}
        format={pct}
      />
      <Slider
        label="Drop rate (simulated missing events)"
        value={state.dropRate}
        onChange={(v) => onChange({ dropRate: v })}
        format={pct}
      />
      <Slider
        label="Late-arrival rate"
        value={state.lateRate}
        onChange={(v) => onChange({ lateRate: v })}
        format={pct}
      />
      <div className="grid grid-cols-2 gap-3">
        <Slider
          label="Late delay min (ms)"
          value={state.lateDelayMsMin}
          min={0}
          max={15000}
          step={250}
          onChange={(v) => onChange({ lateDelayMsMin: v })}
          format={(v) => `${v}ms`}
        />
        <Slider
          label="Late delay max (ms)"
          value={state.lateDelayMsMax}
          min={0}
          max={20000}
          step={250}
          onChange={(v) => onChange({ lateDelayMsMax: v })}
          format={(v) => `${v}ms`}
        />
      </div>
      <div className="flex items-center justify-between pt-2 border-t border-slate-800">
        <label className="flex items-center gap-2 text-sm text-slate-300">
          <input
            type="checkbox"
            checked={state.reorderEnabled}
            onChange={(e) => onChange({ reorderEnabled: e.target.checked })}
            className="accent-emerald-500"
          />
          Reorder events
        </label>
        {state.reorderEnabled && (
          <Slider
            label="Window"
            value={state.reorderWindow}
            min={2}
            max={10}
            step={1}
            onChange={(v) => onChange({ reorderWindow: v })}
            format={(v) => `${v} events`}
          />
        )}
      </div>
      <button
        onClick={() =>
          onChange({
            duplicateRate: 0,
            dropRate: 0,
            lateRate: 0,
            reorderEnabled: false,
          })
        }
        className="w-full text-sm py-2 rounded-md bg-slate-800 hover:bg-slate-700 text-slate-300 transition-colors"
      >
        Reset to happy path
      </button>
    </div>
  );
}
