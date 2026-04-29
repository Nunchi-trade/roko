import './GateVerdictTicker.css';

export interface GateVerdictItem {
  taskId: string;
  gate: string;
  passed: boolean;
  message?: string;
  durationMs: number;
}

export interface GateVerdictTickerProps {
  verdicts: GateVerdictItem[];
  currentTaskId?: string;
}

/** Horizontal strip of gate verdict chips, grouped by task. */
export default function GateVerdictTicker({ verdicts, currentTaskId }: GateVerdictTickerProps) {
  if (verdicts.length === 0) {
    return <div className="gate-verdict-empty">No gate verdicts yet</div>;
  }

  // Group verdicts by task, preserving order
  const groups: { taskId: string; items: GateVerdictItem[] }[] = [];
  let lastTaskId: string | null = null;

  for (const v of verdicts) {
    if (v.taskId !== lastTaskId) {
      groups.push({ taskId: v.taskId, items: [] });
      lastTaskId = v.taskId;
    }
    groups[groups.length - 1].items.push(v);
  }

  return (
    <div className="gate-verdict-ticker">
      {groups.map((group, gi) => (
        <span key={group.taskId} style={{ display: 'contents' }}>
          {gi > 0 && <span className="gate-verdict-divider">|</span>}
          <span className="gate-verdict-divider">{group.taskId.slice(0, 8)}</span>
          {group.items.map((v, vi) => {
            const isCurrent = v.taskId === currentTaskId;
            const cls = [
              'gate-verdict-chip',
              v.passed ? 'pass' : 'fail',
              !isCurrent ? 'dimmed' : '',
            ]
              .filter(Boolean)
              .join(' ');

            return (
              <span
                key={`${v.taskId}-${v.gate}-${vi}`}
                className={cls}
                title={v.message ?? `${v.gate}: ${v.passed ? 'pass' : 'fail'}`}
              >
                <span className="gate-verdict-icon">{v.passed ? '\u2713' : '\u2717'}</span>
                <span className="gate-verdict-name">{v.gate}</span>
                <span className="gate-verdict-ms">{v.durationMs}ms</span>
              </span>
            );
          })}
        </span>
      ))}
    </div>
  );
}
