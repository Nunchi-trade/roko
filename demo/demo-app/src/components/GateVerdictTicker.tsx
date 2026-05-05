import { useEffect, useRef, useState } from 'react';
import './GateVerdictTicker.css';

export interface GateVerdictItem {
  taskId: string;
  gate: string;
  passed: boolean;
  score?: number;
  warning?: boolean;
  message?: string;
  durationMs: number;
}

export interface GateVerdictTickerProps {
  verdicts: GateVerdictItem[];
  currentTaskId?: string;
}

/* ── SVG icon sub-components with path-draw animation ── */

function CheckmarkIcon() {
  return (
    <svg viewBox="0 0 14 14" width="12" height="12" aria-hidden>
      <path
        className="gate-icon-check"
        d="M2.5 7.5 L5.5 10.5 L11.5 3.5"
      />
    </svg>
  );
}

function CrossIcon() {
  return (
    <svg viewBox="0 0 14 14" width="12" height="12" aria-hidden>
      <path className="gate-icon-cross" d="M3 3 L11 11" />
      <path className="gate-icon-cross" d="M11 3 L3 11" />
    </svg>
  );
}

function WarningIcon() {
  return (
    <svg viewBox="0 0 14 14" width="12" height="12" aria-hidden>
      <path
        className="gate-icon-warning"
        d="M7 1.5 L13 12.5 L1 12.5 Z M7 6 L7 9 M7 10.5 L7 11"
      />
    </svg>
  );
}

/* ── Animated score counter ── */

function ScoreCounter({ target }: { target: number }) {
  const [displayed, setDisplayed] = useState(0);
  const rafRef = useRef<number>(0);

  useEffect(() => {
    const start = performance.now();
    const duration = 600;
    const from = 0;
    const to = target;

    function tick(now: number) {
      const elapsed = now - start;
      const progress = Math.min(elapsed / duration, 1);
      // easeOutExpo
      const eased = progress === 1 ? 1 : 1 - Math.pow(2, -10 * progress);
      setDisplayed(Math.round(from + (to - from) * eased));
      if (progress < 1) {
        rafRef.current = requestAnimationFrame(tick);
      }
    }

    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
  }, [target]);

  return <span className="gate-verdict-score">{displayed}%</span>;
}

/* ── Success particles ── */

function PassParticles() {
  return (
    <span className="gate-verdict-particles">
      <span className="gate-verdict-particle" />
      <span className="gate-verdict-particle" />
      <span className="gate-verdict-particle" />
      <span className="gate-verdict-particle" />
    </span>
  );
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
            const verdictType = v.warning ? 'warning' : v.passed ? 'pass' : 'fail';
            const cls = [
              'gate-verdict-chip',
              verdictType,
              !isCurrent ? 'dimmed' : '',
            ]
              .filter(Boolean)
              .join(' ');

            return (
              <span
                key={`${v.taskId}-${v.gate}-${vi}`}
                className={cls}
              >
                {/* SVG icon with path-draw animation */}
                <span className="gate-verdict-icon">
                  {v.warning ? (
                    <WarningIcon />
                  ) : v.passed ? (
                    <CheckmarkIcon />
                  ) : (
                    <CrossIcon />
                  )}
                </span>

                <span className="gate-verdict-name">{v.gate}</span>

                {/* Animated score if present */}
                {v.score != null && <ScoreCounter target={v.score} />}

                <span className="gate-verdict-ms">{v.durationMs}ms</span>

                {/* Success particles on pass */}
                {v.passed && !v.warning && <PassParticles />}

                {/* Tooltip on hover */}
                <span className="gate-verdict-tooltip">
                  {v.message ?? `${v.gate}: ${verdictType}`}
                  {v.score != null && ` (${v.score}%)`}
                  {v.durationMs != null && ` \u2014 ${v.durationMs}ms`}
                </span>
              </span>
            );
          })}
        </span>
      ))}
    </div>
  );
}
