/**
 * ParetoChart — scatter plot wrapper for the Pareto frontier in Analysis.
 * Renders cost vs pass-rate with an optional trend line.
 * Extracted from the Analysis section of Bench.tsx (lines 620-687).
 */
import type { BenchRun, ParetoFrontierResponse } from '../../lib/bench-types';
import { modelColor } from '../../lib/palette';
import Pane from '../Pane';
import ScatterChart from '../Charts/ScatterChart';
import CostRace from '../CostRace';
import './ParetoChart.css';

/* ── Props ── */

export interface ParetoChartProps {
  pareto: ParetoFrontierResponse | null;
  history: BenchRun[];
}

/* ── Component ── */

export function ParetoChart({ pareto, history }: ParetoChartProps) {
  const histWithSummary = history.filter((r) => r.summary);

  // Build scatter data from pareto points or fall back to history
  const pts = pareto?.points ?? [];
  const scatterData =
    pts.length > 0
      ? pts.map((p) => ({
          x: p.cost_usd,
          y: p.pass_rate,
          label: p.label ?? p.run_id.slice(0, 8),
          color: p.provider?.includes('Anthropic')
            ? 'var(--rose-bright)'
            : 'var(--success)',
        }))
      : histWithSummary.map((r) => ({
          x: r.summary!.total_cost_usd,
          y: r.summary!.pass_rate,
          label: r.config.model.split('-').slice(0, 2).join('-'),
          color: modelColor(r.config.model),
        }));

  return (
    <div className="pareto-chart">
      <Pane title="PARETO FRONTIER">
        {scatterData.length > 0 ? (
          <ScatterChart
            points={scatterData}
            xLabel="Cost (USD)"
            yLabel="Pass Rate"
            showTrendLine
            height={400}
          />
        ) : (
          <p className="bench-empty-text">
            Run benchmarks to see the Pareto frontier.
          </p>
        )}
      </Pane>

      {histWithSummary.length > 0 && (
        <>
          <Pane title="MODEL LEADERBOARD">
            <div className="task-table-wrap">
              <table className="task-table">
                <thead>
                  <tr>
                    <th>#</th>
                    <th>Model</th>
                    <th>Runs</th>
                    <th>Avg Pass Rate</th>
                    <th>Avg Cost</th>
                    <th>Avg Duration</th>
                  </tr>
                </thead>
                <tbody>
                  {(() => {
                    const byModel = new Map<string, BenchRun[]>();
                    for (const r of histWithSummary) {
                      const l = byModel.get(r.config.model) ?? [];
                      l.push(r);
                      byModel.set(r.config.model, l);
                    }
                    return [...byModel.entries()]
                      .map(([model, runs]) => ({
                        model,
                        runs: runs.length,
                        avgPass:
                          runs.reduce(
                            (s, r) => s + (r.summary?.pass_rate ?? 0),
                            0,
                          ) / runs.length,
                        avgCost:
                          runs.reduce(
                            (s, r) => s + (r.summary?.total_cost_usd ?? 0),
                            0,
                          ) / runs.length,
                        avgDur:
                          runs.reduce(
                            (s, r) =>
                              s + (r.summary?.total_duration_ms ?? 0),
                            0,
                          ) / runs.length,
                      }))
                      .sort(
                        (a, b) =>
                          b.avgPass - a.avgPass || a.avgCost - b.avgCost,
                      )
                      .map((row, i) => (
                        <tr key={row.model}>
                          <td className="mono">{i + 1}</td>
                          <td className="mono">{row.model}</td>
                          <td className="mono">{row.runs}</td>
                          <td className="mono">
                            {(row.avgPass * 100).toFixed(1)}%
                          </td>
                          <td className="mono">
                            ${row.avgCost.toFixed(3)}
                          </td>
                          <td className="mono">
                            {(row.avgDur / 1000).toFixed(1)}s
                          </td>
                        </tr>
                      ));
                  })()}
                </tbody>
              </table>
            </div>
          </Pane>

          <Pane title="STRATEGY LEADERBOARD">
            <div className="task-table-wrap">
              <table className="task-table">
                <thead>
                  <tr>
                    <th>#</th>
                    <th>Strategy</th>
                    <th>Runs</th>
                    <th>Avg Pass Rate</th>
                    <th>Avg Cost</th>
                  </tr>
                </thead>
                <tbody>
                  {(() => {
                    const byStrat = new Map<string, BenchRun[]>();
                    for (const r of histWithSummary) {
                      const l =
                        byStrat.get(r.config.strategy) ?? [];
                      l.push(r);
                      byStrat.set(r.config.strategy, l);
                    }
                    return [...byStrat.entries()]
                      .map(([strat, runs]) => ({
                        strat,
                        runs: runs.length,
                        avgPass:
                          runs.reduce(
                            (s, r) => s + (r.summary?.pass_rate ?? 0),
                            0,
                          ) / runs.length,
                        avgCost:
                          runs.reduce(
                            (s, r) =>
                              s + (r.summary?.total_cost_usd ?? 0),
                            0,
                          ) / runs.length,
                      }))
                      .sort((a, b) => b.avgPass - a.avgPass)
                      .map((row, i) => (
                        <tr key={row.strat}>
                          <td className="mono">{i + 1}</td>
                          <td>{row.strat.replace(/_/g, ' ')}</td>
                          <td className="mono">{row.runs}</td>
                          <td className="mono">
                            {(row.avgPass * 100).toFixed(1)}%
                          </td>
                          <td className="mono">
                            ${row.avgCost.toFixed(3)}
                          </td>
                        </tr>
                      ));
                  })()}
                </tbody>
              </table>
            </div>
          </Pane>
        </>
      )}

      <Pane title="MODEL COST RACE">
        <CostRace height={300} />
      </Pane>
    </div>
  );
}
