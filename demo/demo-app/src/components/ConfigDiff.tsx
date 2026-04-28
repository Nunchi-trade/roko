import type { BenchRun } from '../lib/bench-types';

interface ConfigDiffProps {
  runA: BenchRun;
  runB: BenchRun;
}

interface DiffRow {
  key: string;
  a: string;
  b: string;
  changed: boolean;
}

export default function ConfigDiff({ runA, runB }: ConfigDiffProps) {
  const rows: DiffRow[] = [
    { key: 'Model', a: runA.config.model, b: runB.config.model, changed: runA.config.model !== runB.config.model },
    { key: 'Strategy', a: runA.config.strategy, b: runB.config.strategy, changed: runA.config.strategy !== runB.config.strategy },
    { key: 'Temperature', a: String(runA.config.temperature ?? '-'), b: String(runB.config.temperature ?? '-'), changed: runA.config.temperature !== runB.config.temperature },
    { key: 'Max Tokens', a: String(runA.config.max_tokens ?? '-'), b: String(runB.config.max_tokens ?? '-'), changed: runA.config.max_tokens !== runB.config.max_tokens },
    { key: 'Timeout', a: `${runA.config.timeout_secs}s`, b: `${runB.config.timeout_secs}s`, changed: runA.config.timeout_secs !== runB.config.timeout_secs },
    { key: 'Retries', a: String(runA.config.retries), b: String(runB.config.retries), changed: runA.config.retries !== runB.config.retries },
    { key: 'Suite', a: runA.suite_name, b: runB.suite_name, changed: runA.suite_id !== runB.suite_id },
  ];

  const passA = runA.summary?.pass_rate ?? 0;
  const passB = runB.summary?.pass_rate ?? 0;
  const costA = runA.summary?.total_cost_usd ?? 0;
  const costB = runB.summary?.total_cost_usd ?? 0;

  return (
    <div className="config-diff">
      <table className="task-table">
        <thead>
          <tr>
            <th>Config</th>
            <th>Run A ({runA.id.slice(0, 8)})</th>
            <th>Run B ({runB.id.slice(0, 8)})</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.key} className={r.changed ? 'diff-changed' : ''}>
              <td className="detail-label">{r.key}</td>
              <td className="mono">{r.a}</td>
              <td className="mono">{r.b}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <div className="config-diff-metrics" style={{ marginTop: 16 }}>
        <table className="task-table">
          <thead>
            <tr>
              <th>Metric</th>
              <th>Run A</th>
              <th>Run B</th>
              <th>Delta</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td className="detail-label">Pass Rate</td>
              <td className="mono">{(passA * 100).toFixed(1)}%</td>
              <td className="mono">{(passB * 100).toFixed(1)}%</td>
              <td className={`mono ${passB > passA ? 'gate-ok' : passB < passA ? 'gate-err' : ''}`}>
                {((passB - passA) * 100).toFixed(1)}%
              </td>
            </tr>
            <tr>
              <td className="detail-label">Cost</td>
              <td className="mono">${costA.toFixed(3)}</td>
              <td className="mono">${costB.toFixed(3)}</td>
              <td className={`mono ${costB < costA ? 'gate-ok' : costB > costA ? 'gate-err' : ''}`}>
                ${(costB - costA).toFixed(3)}
              </td>
            </tr>
            <tr>
              <td className="detail-label">Tasks</td>
              <td className="mono">{runA.summary?.total_tasks ?? '-'}</td>
              <td className="mono">{runB.summary?.total_tasks ?? '-'}</td>
              <td className="mono">
                {(runB.summary?.total_tasks ?? 0) - (runA.summary?.total_tasks ?? 0)}
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  );
}
