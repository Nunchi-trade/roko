import { useState, useMemo, Fragment } from 'react';
import type { BenchTaskResult, TaskStatus } from '../lib/bench-types';
import { handleRowKeyDown } from '../lib/a11y';
import {
  AnimatedRow,
  AnimatedHeaderCell,
  ExpandableDetail,
  TableEmptyState,
} from './AnimatedTable';

interface TaskTableProps {
  results: BenchTaskResult[];
  showDifficulty?: boolean;
  showOutputPreview?: boolean;
}

type SortKey = 'task_name' | 'status' | 'cost_usd' | 'tokens' | 'duration_ms' | 'model' | 'difficulty';

export default function TaskTable({ results, showDifficulty = true, showOutputPreview = true }: TaskTableProps) {
  const [sortKey, setSortKey] = useState<SortKey>('task_name');
  const [sortAsc, setSortAsc] = useState(true);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [filterStatus, setFilterStatus] = useState<TaskStatus | 'all'>('all');
  const [filterText, setFilterText] = useState('');

  const filtered = useMemo(() => {
    let list = results;
    if (filterStatus !== 'all') {
      list = list.filter((r) => r.status === filterStatus);
    }
    if (filterText) {
      const lower = filterText.toLowerCase();
      list = list.filter((r) =>
        r.task_name.toLowerCase().includes(lower) ||
        r.error?.toLowerCase().includes(lower) ||
        r.output_preview?.toLowerCase().includes(lower)
      );
    }
    return list;
  }, [results, filterStatus, filterText]);

  const sorted = useMemo(() => {
    return [...filtered].sort((a, b) => {
      let cmp = 0;
      switch (sortKey) {
        case 'task_name': cmp = a.task_name.localeCompare(b.task_name); break;
        case 'status': cmp = a.status.localeCompare(b.status); break;
        case 'cost_usd': cmp = a.cost_usd - b.cost_usd; break;
        case 'tokens': cmp = (a.tokens_in + a.tokens_out) - (b.tokens_in + b.tokens_out); break;
        case 'duration_ms': cmp = a.duration_ms - b.duration_ms; break;
        case 'model': cmp = a.model.localeCompare(b.model); break;
        case 'difficulty': cmp = (a.difficulty ?? 0) - (b.difficulty ?? 0); break;
      }
      return sortAsc ? cmp : -cmp;
    });
  }, [filtered, sortKey, sortAsc]);

  // Summary
  const totalCost = filtered.reduce((s, r) => s + r.cost_usd, 0);
  const totalTokens = filtered.reduce((s, r) => s + r.tokens_in + r.tokens_out, 0);
  const totalDuration = filtered.reduce((s, r) => s + r.duration_ms, 0);
  const passCount = filtered.filter((r) => r.status === 'pass').length;
  const failCount = filtered.filter((r) => r.status === 'fail').length;

  function handleSort(key: SortKey) {
    if (sortKey === key) setSortAsc(!sortAsc);
    else { setSortKey(key); setSortAsc(true); }
  }

  const colCount = 7 + (showDifficulty ? 1 : 0);

  return (
    <div className="task-table-wrap">
      {/* Filter row */}
      <div className="task-table-filters">
        <div className="task-filter-group">
          {(['all', 'pass', 'fail', 'pending', 'running', 'skipped'] as const).map((s) => (
            <button
              key={s}
              className={`task-filter-btn${filterStatus === s ? ' active' : ''}`}
              onClick={() => setFilterStatus(s)}
            >
              {s === 'all' ? `All (${results.length})` : s.toUpperCase()}
            </button>
          ))}
        </div>
        <input
          type="text"
          className="task-filter-search"
          placeholder="Filter by name, error..."
          value={filterText}
          onChange={(e) => setFilterText(e.target.value)}
        />
      </div>

      <table className="task-table" role="table">
        <thead>
          <tr>
            <AnimatedHeaderCell sortKey="task_name" currentSort={sortKey} ascending={sortAsc} onSort={(k) => handleSort(k as SortKey)}>Task</AnimatedHeaderCell>
            {showDifficulty && <AnimatedHeaderCell sortKey="difficulty" currentSort={sortKey} ascending={sortAsc} onSort={(k) => handleSort(k as SortKey)}>Diff</AnimatedHeaderCell>}
            <AnimatedHeaderCell sortKey="status" currentSort={sortKey} ascending={sortAsc} onSort={(k) => handleSort(k as SortKey)}>Status</AnimatedHeaderCell>
            <AnimatedHeaderCell sortKey="cost_usd" currentSort={sortKey} ascending={sortAsc} onSort={(k) => handleSort(k as SortKey)}>Cost</AnimatedHeaderCell>
            <AnimatedHeaderCell sortKey="tokens" currentSort={sortKey} ascending={sortAsc} onSort={(k) => handleSort(k as SortKey)}>Tokens</AnimatedHeaderCell>
            <AnimatedHeaderCell sortKey="duration_ms" currentSort={sortKey} ascending={sortAsc} onSort={(k) => handleSort(k as SortKey)}>Duration</AnimatedHeaderCell>
            <AnimatedHeaderCell sortKey="model" currentSort={sortKey} ascending={sortAsc} onSort={(k) => handleSort(k as SortKey)}>Model</AnimatedHeaderCell>
            <th>Gates</th>
          </tr>
        </thead>
        <tbody>
          {sorted.length === 0 ? (
            <TableEmptyState colSpan={colCount} message="No matching tasks" />
          ) : (
            sorted.map((r, rowIdx) => (
              <Fragment key={r.task_id}>
                <AnimatedRow
                  index={rowIdx}
                  className={`task-row task-row-${r.status}`}
                  onClick={() => setExpandedId(expandedId === r.task_id ? null : r.task_id)}
                  onKeyDown={(e) => handleRowKeyDown(e, () => setExpandedId(expandedId === r.task_id ? null : r.task_id))}
                  tabIndex={0}
                  role="row"
                  style={{ cursor: 'pointer' }}
                >
                  <td className="task-name">{r.task_name}</td>
                  {showDifficulty && (
                    <td>
                      {r.difficulty != null && (
                        <span className={`diff-badge diff-${r.difficulty}`}>D{r.difficulty}</span>
                      )}
                    </td>
                  )}
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
                        title={`${g.gate}: ${g.passed ? 'passed' : 'failed'}${g.message ? ` — ${g.message}` : ''}`}
                      >
                        {g.gate[0].toUpperCase()}
                      </span>
                    ))}
                  </td>
                </AnimatedRow>
                <ExpandableDetail open={expandedId === r.task_id} colSpan={colCount}>
                  <div className="task-detail" style={{ padding: '12px' }}>
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
                    {showOutputPreview && r.output_preview && (
                      <div className="task-output-preview">
                        <span className="detail-label">Output Preview:</span>
                        <pre className="task-output-code">{r.output_preview}</pre>
                      </div>
                    )}
                    {r.error && <div className="task-error">{r.error}</div>}
                  </div>
                </ExpandableDetail>
              </Fragment>
            ))
          )}
        </tbody>
        <tfoot>
          <tr className="task-table-summary">
            <td className="detail-label">{filtered.length} tasks</td>
            {showDifficulty && <td />}
            <td className="mono">
              <span className="gate-ok">{passCount}P</span>
              {failCount > 0 && <span className="gate-err" style={{ marginLeft: 4 }}>{failCount}F</span>}
            </td>
            <td className="mono">${totalCost.toFixed(3)}</td>
            <td className="mono">{totalTokens.toLocaleString()}</td>
            <td className="mono">{(totalDuration / 1000).toFixed(1)}s</td>
            <td />
            <td />
          </tr>
        </tfoot>
      </table>
    </div>
  );
}
