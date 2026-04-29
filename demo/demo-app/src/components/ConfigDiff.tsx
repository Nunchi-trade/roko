import type { BenchRun } from '../lib/bench-types';

interface ConfigDiffProps {
  runs: BenchRun[];
}

interface DiffRow {
  key: string;
  values: string[];
  changed: boolean;
}

function majority(values: string[]): string {
  const counts = new Map<string, number>();
  for (const v of values) counts.set(v, (counts.get(v) ?? 0) + 1);
  let best = values[0];
  let max = 0;
  for (const [v, c] of counts) {
    if (c > max) { best = v; max = c; }
  }
  return best;
}

export default function ConfigDiff({ runs }: ConfigDiffProps) {
  if (runs.length < 2) return null;

  const configRows: DiffRow[] = [
    { key: 'Model', values: runs.map((r) => r.config.model), changed: false },
    { key: 'Strategy', values: runs.map((r) => r.config.strategy), changed: false },
    { key: 'Temperature', values: runs.map((r) => String(r.config.temperature ?? '-')), changed: false },
    { key: 'Max Tokens', values: runs.map((r) => String(r.config.max_tokens ?? '-')), changed: false },
    { key: 'Timeout', values: runs.map((r) => `${r.config.timeout_secs}s`), changed: false },
    { key: 'Retries', values: runs.map((r) => String(r.config.retries)), changed: false },
    { key: 'Suite', values: runs.map((r) => r.suite_name), changed: false },
  ];

  // Mark changed rows
  for (const row of configRows) {
    row.changed = new Set(row.values).size > 1;
  }

  // Metrics
  const metricRows: { key: string; values: string[]; numValues: number[]; higherBetter: boolean }[] = [
    {
      key: 'Pass Rate',
      values: runs.map((r) => r.summary ? `${(r.summary.pass_rate * 100).toFixed(1)}%` : '-'),
      numValues: runs.map((r) => r.summary?.pass_rate ?? 0),
      higherBetter: true,
    },
    {
      key: 'Total Cost',
      values: runs.map((r) => r.summary ? `$${r.summary.total_cost_usd.toFixed(3)}` : '-'),
      numValues: runs.map((r) => r.summary?.total_cost_usd ?? Infinity),
      higherBetter: false,
    },
    {
      key: 'USD/Success',
      values: runs.map((r) => r.summary ? `$${r.summary.cost_per_success_usd.toFixed(3)}` : '-'),
      numValues: runs.map((r) => r.summary?.cost_per_success_usd ?? Infinity),
      higherBetter: false,
    },
    {
      key: 'Duration',
      values: runs.map((r) => r.summary ? `${(r.summary.total_duration_ms / 1000).toFixed(1)}s` : '-'),
      numValues: runs.map((r) => r.summary?.total_duration_ms ?? Infinity),
      higherBetter: false,
    },
    {
      key: 'Total Tokens',
      values: runs.map((r) => r.summary ? r.summary.total_tokens.toLocaleString() : '-'),
      numValues: runs.map((r) => r.summary?.total_tokens ?? 0),
      higherBetter: false,
    },
    {
      key: 'Token Efficiency',
      values: runs.map((r) => {
        if (!r.summary || r.summary.total_tokens === 0) return '-';
        const eff = r.summary.passed / (r.summary.total_tokens / 1000);
        return `${eff.toFixed(2)}/1K`;
      }),
      numValues: runs.map((r) => {
        if (!r.summary || r.summary.total_tokens === 0) return 0;
        return r.summary.passed / (r.summary.total_tokens / 1000);
      }),
      higherBetter: true,
    },
    {
      key: 'Gate Pass Rate',
      values: runs.map((r) => {
        if (!r.results.length) return '-';
        let passed = 0, total = 0;
        for (const res of r.results) {
          for (const g of res.gate_verdicts) {
            total++;
            if (g.passed) passed++;
          }
        }
        return total > 0 ? `${((passed / total) * 100).toFixed(1)}%` : '-';
      }),
      numValues: runs.map((r) => {
        let passed = 0, total = 0;
        for (const res of r.results) {
          for (const g of res.gate_verdicts) {
            total++;
            if (g.passed) passed++;
          }
        }
        return total > 0 ? passed / total : 0;
      }),
      higherBetter: true,
    },
  ];

  return (
    <div className="config-diff">
      <table className="task-table">
        <thead>
          <tr>
            <th>Config</th>
            {runs.map((r) => (
              <th key={r.id}>Run {r.id.slice(0, 8)}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {configRows.map((row) => {
            const maj = majority(row.values);
            return (
              <tr key={row.key} className={row.changed ? 'diff-changed' : ''}>
                <td className="detail-label">{row.key}</td>
                {row.values.map((v, i) => (
                  <td
                    key={i}
                    className={`mono${row.changed && v !== maj ? ' diff-highlight' : ''}`}
                  >
                    {v}
                  </td>
                ))}
              </tr>
            );
          })}
        </tbody>
      </table>

      <div className="config-diff-metrics" style={{ marginTop: 16 }}>
        <table className="task-table">
          <thead>
            <tr>
              <th>Metric</th>
              {runs.map((r) => (
                <th key={r.id}>Run {r.id.slice(0, 8)}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {metricRows.map((row) => {
              const best = row.higherBetter
                ? Math.max(...row.numValues)
                : Math.min(...row.numValues);

              return (
                <tr key={row.key}>
                  <td className="detail-label">{row.key}</td>
                  {row.values.map((v, i) => (
                    <td
                      key={i}
                      className={`mono${row.numValues[i] === best && v !== '-' ? ' gate-ok' : ''}`}
                      style={row.numValues[i] === best && v !== '-' ? { fontWeight: 700 } : undefined}
                    >
                      {v}
                    </td>
                  ))}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
