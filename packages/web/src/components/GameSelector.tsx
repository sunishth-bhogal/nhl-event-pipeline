import type { GameSummary } from "../lib/useConsumerFeed";

interface Props {
  games: GameSummary[];
  selected: string | null;
  onSelect: (gameId: string) => void;
}

export function GameSelector({ games, selected, onSelect }: Props) {
  if (games.length === 0) {
    return <div className="text-sm text-slate-500">No games ingested yet.</div>;
  }
  return (
    <select
      className="bg-slate-800 border border-slate-700 rounded-md px-2 py-1.5 text-sm font-mono"
      value={selected ?? ""}
      onChange={(e) => onSelect(e.target.value)}
    >
      {games.map((g) => (
        <option key={g.gameId} value={g.gameId}>
          {g.gameId} ({g.playCount} plays)
        </option>
      ))}
    </select>
  );
}
