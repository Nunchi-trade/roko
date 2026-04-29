import { useState, Fragment } from 'react';
import type { BenchTaskResult } from '../lib/bench-types';

interface TaskTableProps {
  results: BenchTaskResult[];
}

type SortKey = 'task_name' | 'status' | 'cost_usd' | 'tokens' | 'duration_ms' | 'model';

export default function TaskTable({ results }: TaskTableProps) {
  const [sortKey, setSortKey] = useState<SortKey>('task_name');
  const [sortAsc, setSortAsc] = useState(true);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const sorted = [...results].sort((a, b) => {
    let cmp = 0;
    switch (sortKey) {
      case 'task_name': cmp = a.task_name.localeCompare(b.task_name); break;
      case 'status': cmp = a.status.localeCompare(b.status); break;
      case 'cost_usd': cmp = a.cost_usd - b.cost_usd; break;
      case 'tokens': cmp = (a.tokens_in + a.tokens_out) - (b.tokens_in + b.tokens_out); break;
      case 'duration_ms': cmp = a.duration_ms - b.duration_ms; break;
      case 'model': cmp = a.model.localeCompare(b.model); break;
    }
    return sortAsc ? cmp : -cmp;
  });

  function handleSort(key: SortKey) {
    if (sortKey === key) setSortAsc(!sortAsc);
    else { setSortKey(key); setSortAsc(true); }
  }

  const arrow = (key: SortKey) => sortKey === key ? (sortAsc ? ' \u2191' : ' \u2193') : '';

  return (
    <div className="task-table-wrap">
      <table className="task-table">
        <thead>
          <tr>
            <th onClick={() => handleSort('task_name')}>Task{arrow('task_name')}</th>
            <th onClick={() => handleSort('status')}>Status{arrow('status')}</th>
            <th onClick={() => handleSort('cost_usd')}>Cost{arrow('cost_usd')}</th>
            <th onClick={() => handleSort('tokens')}>Tokens{arrow('tokens')}</th>
            <th onClick={() => handleSort('duration_ms')}>Duration{arrow('duration_ms')}</th>
            <th onClick={() => handleSort('model')}>Model{arrow('model')}</th>
            <th>Gates</th>
          </tr>
        </thead>
        <tbody>
          {sorted.map((r) => (
            <Fragment key={r.task_id}>
              <tr
                className={`task-row task-row-${r.status}`}
                onClick={() => setExpandedId(expandedId === r.task_id ? null : r.task_id)}
                style={{ cursor: 'pointer' }}
              >
                <td className="task-name">{r.task_name}</td>
                <td>
                  <span className={`status-badge status-${r.status}`}>
                    {r.status.toUpperCase()}
                  </span>
                </td>
                <td className="mono">${r.cost_usd.toFixed(3)}</td>
                <td className="mono">{(r.tokens_in + r.tokens_out).toLocaleString()}</td>
                <td className="mono">{(r.duration_ms / 1000).toFixed(1)}s</td>
                <td className="mono">{r.model.split('-').slice(0, 2).join('-')}</td>
                <td>
                  {r.gate_verdicts.map((g) => (
                    <span
                      key={g.gate}
                      className={`gate-pill gate-${g.passed ? 'pass' : 'fail'}`}
                      title={`${g.gate}: ${g.passed ? 'passed' : 'failed'}`}
                    >
                      {g.gate[0].toUpperCase()}
                    </span>
                  ))}
                </td>
              </tr>
              {expandedId === r.task_id && (
                <tr key={`${r.task_id}-detail`} className="task-detail-row">
                  <td colSpan={7}>
                    <div className="task-detail">
                      <div className="task-detail-grid">
                        <div><span className="detail-label">Tokens in:</span> {r.tokens_in.toLocaleString()}</div>
                        <div><span className="detail-label">Tokens out:</span> {r.tokens_out.toLocaleString()}</div>
                        <div><span className="detail-label">Retries:</span> {r.retries_used}</div>
                        <div><span className="detail-label">Model:</span> {r.model}</div>
                      </div>
                      {r.gate_verdicts.length > 0 && (
                        <div className="task-detail-gates">
                          {r.gate_verdicts.map((g) => (
                            <div key={g.gate} className={`gate-detail gate-${g.passed ? 'pass' : 'fail'}`}>
                              <span className="gate-name">{g.gate}</span>
                              <span className={g.passed ? 'gate-ok' : 'gate-err'}>
                                {g.passed ? 'PASSED' : 'FAILED'}
                              </span>
                              {g.duration_ms != null && (
                                <span className="gate-time">{g.duration_ms}ms</span>
                              )}
                              {g.message && <span className="gate-msg">{g.message}</span>}
                            </div>
                          ))}
                        </div>
                      )}
                      {r.error && <div className="task-error">{r.error}</div>}
                    </div>
                  </td>
                </tr>
              )}
            </Fragment>
          ))}
        </tbody>
      </table>
    </div>
  );
}
