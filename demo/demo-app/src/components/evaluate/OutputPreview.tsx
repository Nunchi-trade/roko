/**
 * OutputPreview — expandable list of task outputs (failed auto-expanded).
 * Extracted from BenchRunDetail.tsx (OutputPreviewPanel).
 */
import { useState, useEffect } from 'react';
import type { BenchTaskResult } from '../../lib/bench-types';
import './OutputPreview.css';

/* ── GateBadge (inline, small SVG path-draw checkmark / X) ── */

function useStagger(delayMs: number): boolean {
  const [ready, setReady] = useState(false);
  useEffect(() => {
    const t = setTimeout(() => setReady(true), delayMs);
    return () => clearTimeout(t);
  }, [delayMs]);
  return ready;
}

function GateBadge({ passed, gate, delay = 0 }: { passed: boolean; gate: string; delay?: number }) {
  const visible = useStagger(delay);

  return (
    <span
      className={`gate-pill ${passed ? 'gate-pass' : 'gate-fail'} gate-badge-pill ${visible ? 'gate-badge-pill--visible' : 'gate-badge-pill--hidden'}`}
      title={`${gate}: ${passed ? 'PASS' : 'FAIL'}`}
    >
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
        {passed ? (
          <path
            d="M2 5.5 L4 7.5 L8 3"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
            style={{
              strokeDasharray: 12,
              strokeDashoffset: visible ? 0 : 12,
              transition: `stroke-dashoffset 500ms var(--ease) ${delay + 100}ms`,
            }}
          />
        ) : (
          <>
            <path
              d="M2.5 2.5 L7.5 7.5"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              style={{
                strokeDasharray: 8,
                strokeDashoffset: visible ? 0 : 8,
                transition: `stroke-dashoffset 400ms var(--ease) ${delay + 80}ms`,
              }}
            />
            <path
              d="M7.5 2.5 L2.5 7.5"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              style={{
                strokeDasharray: 8,
                strokeDashoffset: visible ? 0 : 8,
                transition: `stroke-dashoffset 400ms var(--ease) ${delay + 160}ms`,
              }}
            />
          </>
        )}
      </svg>
    </span>
  );
}

/* ── Props ── */

export interface OutputPreviewProps {
  results: BenchTaskResult[];
}

/* ── Component ── */

export function OutputPreview({ results }: OutputPreviewProps) {
  const failedWithOutput = results.filter((r) => r.status === 'fail' && (r.output_preview || r.error));
  const passedWithOutput = results.filter((r) => r.status === 'pass' && r.output_preview);

  const [expandedIds, setExpandedIds] = useState<Set<string>>(() => {
    // Auto-expand failed tasks
    return new Set(failedWithOutput.map((r) => r.task_id));
  });

  const toggle = (id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  if (failedWithOutput.length === 0 && passedWithOutput.length === 0) {
    return <p className="bench-empty-text">No output previews available.</p>;
  }

  const allTasks = [...failedWithOutput, ...passedWithOutput];

  return (
    <div className="output-preview-list">
      {allTasks.map((r, i) => {
        const isExpanded = expandedIds.has(r.task_id);
        return (
          <div
            key={r.task_id}
            className="output-preview-card"
            style={{ animation: `fadeUp 400ms var(--ease) ${i * 60}ms both` }}
          >
            <button
              onClick={() => toggle(r.task_id)}
              className="output-preview-header"
            >
              <span className={`output-preview-chevron${isExpanded ? ' output-preview-chevron--open' : ''}`}>
                {'\u25B6'}
              </span>
              <span className={`status-badge status-${r.status} status-badge-lg`}>
                {r.status.toUpperCase()}
              </span>
              <span className="output-preview-name">{r.task_name}</span>
              {r.gate_verdicts.length > 0 && (
                <span className="output-preview-gates">
                  {r.gate_verdicts.map((g, gi) => (
                    <GateBadge
                      key={g.gate}
                      gate={g.gate}
                      passed={g.passed}
                      delay={isExpanded ? gi * 80 : 0}
                    />
                  ))}
                </span>
              )}
            </button>
            <div className={`output-preview-body ${isExpanded ? 'output-preview-body--expanded' : 'output-preview-body--collapsed'}`}>
              <div className="output-preview-content">
                {r.error && (
                  <div className="task-error output-preview-error">{r.error}</div>
                )}
                {r.output_preview && (
                  <pre
                    className="task-output-code output-preview-code"
                    style={{ animation: isExpanded ? 'fadeIn 500ms var(--ease) 200ms both' : 'none' }}
                  >
                    {r.output_preview}
                  </pre>
                )}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}
