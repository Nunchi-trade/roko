import type { AgentIdentity } from '../Spectre/AgentIdentity';
import TerminalPane from './TerminalPane';
import './TerminalGrid.css';

export interface TerminalGridEntry {
  sessionId: string;
  label: string;
  agent?: AgentIdentity;
}

interface TerminalGridProps {
  entries: TerminalGridEntry[];
  maxColumns?: number; // default 2
}

/**
 * Compute the column count for a given number of entries:
 *   1       -> 1 column  (full width)
 *   2       -> 2 columns (side by side)
 *   3-4     -> 2 columns (2x2 grid)
 *   5-6     -> 3 columns (3x2)
 *   7+      -> capped at maxColumns
 */
function gridColumns(count: number, maxColumns: number): number {
  if (count <= 1) return 1;
  if (count <= 4) return Math.min(2, maxColumns);
  if (count <= 6) return Math.min(3, maxColumns);
  return maxColumns;
}

export default function TerminalGrid({ entries, maxColumns = 2 }: TerminalGridProps) {
  const cols = gridColumns(entries.length, maxColumns);

  return (
    <div
      className="terminal-grid"
      style={{ gridTemplateColumns: `repeat(${cols}, 1fr)` }}
    >
      {entries.map((entry) => (
        <TerminalPane
          key={entry.sessionId}
          sessionId={entry.sessionId}
          label={entry.label}
          agent={entry.agent}
        />
      ))}
    </div>
  );
}
