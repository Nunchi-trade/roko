/**
 * RunListTab — History/run-list tab for the Evaluate scene.
 * Extracted from the History section of Bench.tsx (lines 507-560).
 */
import { useState } from 'react';
import { Link } from 'react-router';
import type { BenchRun } from '../../lib/bench-types';
import Pane from '../Pane';
import './RunListTab.css';

/* ── Props ── */

export interface RunListTabProps {
  history: BenchRun[];
  historyLoading: boolean;
  exportRun: (id: string) => void;
  importRun: (file: File) => void;
  onCompare: (ids: string[]) => void;
}

/* ── Component ── */

export function RunListTab({
  history,
  historyLoading,
  exportRun,
  importRun,
  onCompare,
}: RunListTabProps) {
  // Multi-select for comparison
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const toggleSelect = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else if (next.size < 6) next.add(id);
      return next;
    });
  };

  // Filters
  const [filter, setFilter] = useState({ suite: '', model: '', status: '' });
  const filtered = history.filter((r) => {
    if (filter.suite && r.suite_id !== filter.suite) return false;
    if (filter.model && r.config.model !== filter.model) return false;
    if (filter.status && r.status !== filter.status) return false;
    return true;
  });

  return (
    <div className="run-list-tab">
      <div className="run-list-tab__toolbar">
        <div className="run-list-tab__filters">
          <select
            className="config-input"
            style={{ maxWidth: 160 }}
            value={filter.suite}
            onChange={(e) => setFilter({ ...filter, suite: e.target.value })}
          >
            <option value="">All suites</option>
            {[...new Set(history.map((r) => r.suite_id))].map((sid) => (
              <option key={sid} value={sid}>
                {history.find((r) => r.suite_id === sid)?.suite_name ?? sid}
              </option>
            ))}
          </select>
          <select
            className="config-input"
            style={{ maxWidth: 160 }}
            value={filter.model}
            onChange={(e) => setFilter({ ...filter, model: e.target.value })}
          >
            <option value="">All models</option>
            {[...new Set(history.map((r) => r.config.model))].map((m) => (
              <option key={m} value={m}>
                {m.split('-').slice(0, 2).join('-')}
              </option>
            ))}
          </select>
        </div>
        <div className="run-list-tab__actions">
          {selected.size >= 2 && (
            <button
              className="btn btn-sm"
              onClick={() => onCompare([...selected])}
            >
              Compare ({selected.size})
            </button>
          )}
          <label className="btn btn-sm" style={{ cursor: 'pointer' }}>
            Import
            <input
              type="file"
              accept=".json"
              style={{ display: 'none' }}
              onChange={(e) => {
                const file = e.target.files?.[0];
                if (file) importRun(file);
                e.target.value = '';
              }}
            />
          </label>
        </div>
      </div>

      {historyLoading ? (
        <div className="bench-skeleton" style={{ height: 200 }} />
      ) : filtered.length === 0 ? (
        <div className="bench-empty--no-runs">
          <p className="bench-empty-text">No runs recorded yet.</p>
        </div>
      ) : (
        <Pane title={`RUN HISTORY (${filtered.length})`}>
          <div className="task-table-wrap">
            <table className="task-table">
              <thead>
                <tr>
                  <th style={{ width: 32 }}></th>
                  <th>Date</th>
                  <th>Suite</th>
                  <th>Model</th>
                  <th>Strategy</th>
                  <th>Pass Rate</th>
                  <th>Cost</th>
                  <th>Duration</th>
                  <th>Status</th>
                  <th>Actions</th>
                </tr>
              </thead>
              <tbody>
                {filtered.map((run) => (
                  <tr
                    key={run.id}
                    className={selected.has(run.id) ? 'diff-changed' : ''}
                  >
                    <td>
                      <input
                        type="checkbox"
                        checked={selected.has(run.id)}
                        onChange={() => toggleSelect(run.id)}
                        style={{ accentColor: 'var(--rose-bright)' }}
                      />
                    </td>
                    <td className="mono">
                      {new Date(run.started_at).toLocaleDateString()}
                    </td>
                    <td>{run.suite_name}</td>
                    <td className="mono">
                      {run.config.model.split('-').slice(0, 2).join('-')}
                    </td>
                    <td>{run.config.strategy.replace(/_/g, ' ')}</td>
                    <td className="mono">
                      {run.summary
                        ? `${(run.summary.pass_rate * 100).toFixed(0)}%`
                        : '-'}
                    </td>
                    <td className="mono">
                      {run.summary
                        ? `$${run.summary.total_cost_usd.toFixed(3)}`
                        : '-'}
                    </td>
                    <td className="mono">
                      {run.summary
                        ? `${(run.summary.total_duration_ms / 1000).toFixed(1)}s`
                        : '-'}
                    </td>
                    <td>
                      <span
                        className={`status-badge status-${run.status === 'completed' ? 'pass' : run.status}`}
                      >
                        {run.status.toUpperCase()}
                      </span>
                    </td>
                    <td style={{ display: 'flex', gap: 4 }}>
                      <Link
                        to={`/bench/run/${run.id}`}
                        className="btn btn-sm"
                        style={{
                          textDecoration: 'none',
                          fontSize: 13,
                          padding: '2px 6px',
                        }}
                      >
                        View
                      </Link>
                      <button
                        className="btn btn-sm"
                        onClick={() => exportRun(run.id)}
                        style={{ fontSize: 13, padding: '2px 6px' }}
                      >
                        Export
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Pane>
      )}
    </div>
  );
}
