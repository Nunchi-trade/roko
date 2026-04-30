/**
 * OutputPreview — expandable list of task outputs (failed auto-expanded).
 * Extracted from BenchRunDetail.tsx (OutputPreviewPanel).
 */
import { useState } from 'react';
import type { BenchTaskResult } from '../../lib/bench-types';
import './OutputPreview.css';

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
    <div className="output-preview">
      {allTasks.map((r) => (
        <div key={r.task_id} className="output-preview__item">
          <button
            onClick={() => toggle(r.task_id)}
            className="output-preview__toggle"
          >
            <span className="output-preview__arrow">
              {expandedIds.has(r.task_id) ? '\u25BC' : '\u25B6'}
            </span>
            <span className={`status-badge status-${r.status}`} style={{ fontSize: 'var(--text-sm)' }}>
              {r.status.toUpperCase()}
            </span>
            <span className="output-preview__task-name">{r.task_name}</span>
          </button>
          {expandedIds.has(r.task_id) && (
            <div className="output-preview__body">
              {r.error && (
                <div className="task-error" style={{ marginTop: 'var(--sp-2)' }}>{r.error}</div>
              )}
              {r.output_preview && (
                <pre className="output-preview__code">
                  {r.output_preview}
                </pre>
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
