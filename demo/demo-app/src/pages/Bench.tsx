import { useState } from 'react';
import { useBench } from '../hooks/useBench';
import { useRokoConfig } from '../hooks/useRokoConfig';
import type { AgentStrategy } from '../lib/bench-types';
import Pane from '../components/Pane';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import BarChart from '../components/Charts/BarChart';
import CostChart from '../components/Charts/CostChart';
import ParetoChart from '../components/Charts/ParetoChart';
import type { ParetoPoint } from '../components/Charts/ParetoChart';
import SuiteSelector from '../components/SuiteSelector';
import TaskTable from '../components/TaskTable';
import './Bench.css';

type Tab = 'configure' | 'live' | 'results' | 'history' | 'pareto';

const STRATEGIES: { id: AgentStrategy; label: string; desc: string }[] = [
  { id: 'minimal', label: 'Minimal', desc: 'Basic agent, no enrichment' },
  { id: 'context_enriched', label: 'Context-Enriched', desc: 'With context bidders' },
  { id: 'neuro_augmented', label: 'Neuro-Augmented', desc: 'With knowledge store' },
  { id: 'full_cascade', label: 'Full Cascade', desc: 'Complete pipeline with replan' },
];

const TABS: { id: Tab; label: string }[] = [
  { id: 'configure', label: 'Configure' },
  { id: 'live', label: 'Live' },
  { id: 'results', label: 'Results' },
  { id: 'history', label: 'History' },
  { id: 'pareto', label: 'Pareto' },
];

export default function Bench() {
  const [tab, setTab] = useState<Tab>('configure');
  const bench = useBench();
  const {
    config, setConfig,
    selectedSuiteId, setSelectedSuiteId, selectedSuite,
    suites, history,
    activeRun, activeRunSummary, feed,
    startRun, cancelRun, exportRun, importRun,
    lastCompletedRun,
  } = bench;
  const { defaultModel, defaultBackend } = useRokoConfig();

  // Compute hero stats from history
  const totalRuns = history.length;
  const avgPassRate = history.length > 0
    ? history.reduce((s, r) => s + (r.summary?.pass_rate ?? 0), 0) / history.length
    : 0;
  const totalCost = history.reduce((s, r) => s + (r.summary?.total_cost_usd ?? 0), 0);

  // Results for the results tab — from last completed run or active run
  const displayResults = activeRun?.results ?? (lastCompletedRun && 'results' in lastCompletedRun ? (lastCompletedRun as { results: typeof activeRun extends null ? never : NonNullable<typeof activeRun>['results'] }).results : []);
  const displaySummary = activeRunSummary ?? (lastCompletedRun && 'summary' in lastCompletedRun ? (lastCompletedRun as { summary?: NonNullable<typeof activeRunSummary> }).summary : undefined);

  // Pareto data: one point per completed run
  const paretoData: ParetoPoint[] = history
    .filter((r) => r.summary)
    .map((r) => ({
      label: r.config.model.split('-').slice(0, 2).join('-'),
      cost: r.summary!.total_cost_usd,
      passRate: r.summary!.pass_rate,
      color: r.config.model.includes('haiku') ? 'var(--bone-bright)'
        : r.config.model.includes('sonnet') ? 'var(--rose-bright)'
        : r.config.model.includes('opus') ? 'var(--dream-bright)'
        : 'var(--success)',
    }));

  return (
    <div className="bench-page">
      <div className="bench-hero">
        <div className="bench-hero-header">
          <h1 className="bench-page-title">Benchmark Lab</h1>
          <p className="bench-page-sub">Configure, run, and analyze agent evaluations</p>
        </div>
        <div className="bench-hero-stats">
          <Mosaic columns={4}>
            <MosaicCell label="TOTAL RUNS" value={String(totalRuns || '-')} color="bone" />
            <MosaicCell label="AVG PASS RATE" value={avgPassRate > 0 ? `${(avgPassRate * 100).toFixed(0)}%` : '-'} color="success" />
            <MosaicCell label="TOTAL COST" value={totalCost > 0 ? `$${totalCost.toFixed(2)}` : '-'} color="warning" />
            <MosaicCell label="SUITES" value={String(suites.length)} color="rose" />
          </Mosaic>
        </div>
      </div>

      <div className="bench-tabs">
        {TABS.map((t) => (
          <button key={t.id} className={`bench-tab${tab === t.id ? ' active' : ''}`} onClick={() => setTab(t.id)}>
            {t.label}
            {t.id === 'live' && activeRun?.status === 'running' && (
              <span className="bench-tab-dot" />
            )}
          </button>
        ))}
      </div>

      <div className="bench-body">
        {/* ── Configure Tab ── */}
        {tab === 'configure' && (
          <div className="bench-config">
            <Pane title="TEST SUITE">
              <SuiteSelector
                suites={suites}
                value={selectedSuiteId}
                onChange={setSelectedSuiteId}
              />
            </Pane>

            <Pane title="AGENT STRATEGY">
              <div className="config-cards">
                {STRATEGIES.map((s) => (
                  <button
                    key={s.id}
                    className={`config-card${config.strategy === s.id ? ' selected' : ''}`}
                    onClick={() => setConfig({ ...config, strategy: s.id })}
                  >
                    <span className="card-label">{s.label}</span>
                    <span className="card-desc">{s.desc}</span>
                  </button>
                ))}
              </div>
            </Pane>

            <Pane title="MODEL">
              <div className="bench-model-display">
                <span className="bench-model-value">{defaultModel || '—'}</span>
                <span className="bench-model-provider">{defaultBackend || '—'}</span>
                <span className="bench-model-hint">Change via config pill (bottom-right)</span>
              </div>
            </Pane>

            <Pane title="PARAMETERS">
              <div className="config-params">
                <label className="param-row">
                  <span className="param-label">Temperature</span>
                  <input
                    type="range"
                    min="0"
                    max="1"
                    step="0.1"
                    value={config.temperature}
                    onChange={(e) => setConfig({ ...config, temperature: Number(e.target.value) })}
                    className="param-slider"
                  />
                  <span className="param-value">{config.temperature}</span>
                </label>
                <label className="param-row">
                  <span className="param-label">Max Tokens</span>
                  <input
                    type="number"
                    className="config-input"
                    value={config.maxTokens}
                    onChange={(e) => setConfig({ ...config, maxTokens: Number(e.target.value) })}
                    style={{ maxWidth: 120 }}
                  />
                </label>
                <label className="param-row">
                  <span className="param-label">Timeout (s)</span>
                  <input
                    type="number"
                    className="config-input"
                    value={config.timeoutSecs}
                    onChange={(e) => setConfig({ ...config, timeoutSecs: Number(e.target.value) })}
                    style={{ maxWidth: 120 }}
                  />
                </label>
                <label className="param-row">
                  <span className="param-label">Retries</span>
                  <input
                    type="number"
                    className="config-input"
                    min="0"
                    max="3"
                    value={config.retries}
                    onChange={(e) => setConfig({ ...config, retries: Number(e.target.value) })}
                    style={{ maxWidth: 80 }}
                  />
                </label>
                <div className="param-row">
                  <span className="param-label">Gates</span>
                  <div className="gate-toggles">
                    {(['compile', 'test', 'clippy', 'diff'] as const).map((g) => (
                      <label key={g} className="gate-toggle">
                        <input
                          type="checkbox"
                          checked={config.gates[g]}
                          onChange={(e) => setConfig({
                            ...config,
                            gates: { ...config.gates, [g]: e.target.checked },
                          })}
                        />
                        <span>{g}</span>
                      </label>
                    ))}
                  </div>
                </div>
              </div>
            </Pane>

            {selectedSuite && (
              <div className="bench-cost-estimate">
                <span className="cost-label">Estimated cost:</span>
                <span className="cost-value">${selectedSuite.estimated_cost_usd.toFixed(2)}</span>
                <span className="cost-detail">
                  ({selectedSuite.tasks.length} tasks, {config.strategy.replace(/_/g, ' ')})
                </span>
              </div>
            )}

            <div className="bench-run-btn" style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
              <button
                className="btn"
                onClick={() => { startRun(defaultModel, defaultBackend); setTab('live'); }}
                disabled={activeRun?.status === 'running'}
              >
                {activeRun?.status === 'running' ? 'Running...' : 'Run Benchmark'}
              </button>
            </div>
          </div>
        )}

        {/* ── Live Tab ── */}
        {tab === 'live' && (
          <div className="bench-live">
            {!activeRun ? (
              <div className="bench-empty">
                <p className="bench-empty-text">No active run. Go to Configure to start one.</p>
              </div>
            ) : (
              <>
                <div className="bench-live-header">
                  <span className={`benchlive-dot${activeRun.status === 'running' ? '' : ' disconnected'}`} />
                  <span className="bench-live-status">
                    {activeRun.status === 'running' ? 'RUNNING' : activeRun.status.toUpperCase()}
                  </span>
                  <span className="bench-live-progress">
                    {activeRun.progress}/{activeRun.total}
                  </span>
                  <span className="bench-live-cost">${activeRun.costSoFar.toFixed(3)}</span>
                  {activeRun.status === 'running' && (
                    <button className="btn btn-sm" onClick={cancelRun} style={{ marginLeft: 'auto' }}>
                      Cancel
                    </button>
                  )}
                </div>

                <div className="bench-live-progress-bar">
                  <div
                    className="bench-live-progress-fill"
                    style={{ width: `${activeRun.total > 0 ? (activeRun.progress / activeRun.total) * 100 : 0}%` }}
                  />
                </div>

                <div className="benchlive-grid">
                  <Pane title="TASK GRID">
                    <div className="task-grid">
                      {Array.from({ length: activeRun.total }, (_, i) => {
                        const result = activeRun.results[i];
                        const status = result
                          ? result.status
                          : i === activeRun.results.length && activeRun.status === 'running'
                            ? 'running'
                            : 'pending';
                        return (
                          <div
                            key={i}
                            className={`task-cell task-${status}`}
                            title={result ? result.task_name : `Task ${i + 1}`}
                          />
                        );
                      })}
                    </div>
                  </Pane>

                  <Pane title="COST CHART">
                    <CostChart
                      data={activeRun.results.map((r, i) => ({
                        label: `T${i + 1}`,
                        value: r.cost_usd,
                      }))}
                      height={260}
                      color="var(--bone)"
                    />
                  </Pane>

                  <Pane title="ACTIVITY FEED">
                    <div className="feed-list">
                      {feed.map((item, i) => (
                        <div key={i} className={`feed-item feed-${item.type}`}>
                          <span className="feed-ts">{item.ts}</span>
                          <span className="feed-text">{item.text}</span>
                          {item.cost != null && <span className="feed-cost">${item.cost.toFixed(3)}</span>}
                        </div>
                      ))}
                    </div>
                  </Pane>
                </div>
              </>
            )}
          </div>
        )}

        {/* ── Results Tab ── */}
        {tab === 'results' && (
          <div className="bench-results">
            {displayResults.length === 0 ? (
              <div className="bench-empty">
                <p className="bench-empty-text">No results yet. Run a benchmark to see results here.</p>
              </div>
            ) : (
              <>
                <div className="bench-results-stats">
                  <Mosaic columns={4}>
                    <MosaicCell
                      label="PASS RATE"
                      value={displaySummary ? `${(displaySummary.pass_rate * 100).toFixed(0)}%` : '-'}
                      color="success"
                    />
                    <MosaicCell
                      label="TOTAL COST"
                      value={displaySummary ? `$${displaySummary.total_cost_usd.toFixed(3)}` : '-'}
                      color="warning"
                    />
                    <MosaicCell
                      label="USD/SUCCESS"
                      value={displaySummary ? `$${displaySummary.cost_per_success_usd.toFixed(3)}` : '-'}
                      color="bone"
                      mono
                    />
                    <MosaicCell
                      label="AVG DURATION"
                      value={displaySummary ? `${(displaySummary.avg_duration_ms / 1000).toFixed(1)}s` : '-'}
                      color="dream"
                      mono
                    />
                  </Mosaic>
                </div>

                <Pane title="TASK RESULTS">
                  <TaskTable results={displayResults} />
                </Pane>

                <Pane title="COST PER TASK">
                  <BarChart
                    data={displayResults.slice(-30).map((r) => ({
                      label: r.task_name.slice(0, 20),
                      value: r.cost_usd,
                      color: r.status === 'pass' ? 'var(--success)' : 'var(--rose-dim)',
                    }))}
                    height={250}
                  />
                </Pane>

                {displayResults.length > 0 && (
                  <Pane title="GATE BREAKDOWN">
                    <div className="gate-breakdown">
                      {(() => {
                        const gateCounts: Record<string, { passed: number; total: number }> = {};
                        for (const r of displayResults) {
                          for (const g of r.gate_verdicts) {
                            if (!gateCounts[g.gate]) gateCounts[g.gate] = { passed: 0, total: 0 };
                            gateCounts[g.gate].total++;
                            if (g.passed) gateCounts[g.gate].passed++;
                          }
                        }
                        return Object.entries(gateCounts).map(([gate, { passed, total }]) => (
                          <div key={gate} className="gate-breakdown-item">
                            <span className="gate-breakdown-name">{gate}</span>
                            <div className="gate-breakdown-bar">
                              <div
                                className="gate-breakdown-fill"
                                style={{ width: `${(passed / total) * 100}%` }}
                              />
                            </div>
                            <span className="gate-breakdown-pct">
                              {((passed / total) * 100).toFixed(0)}%
                            </span>
                          </div>
                        ));
                      })()}
                    </div>
                  </Pane>
                )}
              </>
            )}
          </div>
        )}

        {/* ── History Tab ── */}
        {tab === 'history' && (
          <div className="bench-history">
            <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: 8 }}>
              <label className="btn btn-sm" style={{ cursor: 'pointer', fontSize: 11, padding: '4px 12px' }}>
                Import Run
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

            {history.length === 0 ? (
              <div className="bench-empty">
                <p className="bench-empty-text">No runs recorded yet.</p>
              </div>
            ) : (
              <>
                <Pane title="RUN HISTORY">
                  <div className="task-table-wrap">
                    <table className="task-table">
                      <thead>
                        <tr>
                          <th>Date</th>
                          <th>Suite</th>
                          <th>Model</th>
                          <th>Strategy</th>
                          <th>Pass Rate</th>
                          <th>Cost</th>
                          <th>Status</th>
                          <th>Actions</th>
                        </tr>
                      </thead>
                      <tbody>
                        {history.map((run) => (
                          <tr key={run.id}>
                            <td className="mono">{new Date(run.started_at).toLocaleDateString()}</td>
                            <td>{run.suite_name}</td>
                            <td className="mono">{run.config.model.split('-').slice(0, 2).join('-')}</td>
                            <td>{run.config.strategy.replace(/_/g, ' ')}</td>
                            <td className="mono">
                              {run.summary ? `${(run.summary.pass_rate * 100).toFixed(0)}%` : '-'}
                            </td>
                            <td className="mono">
                              {run.summary ? `$${run.summary.total_cost_usd.toFixed(3)}` : '-'}
                            </td>
                            <td>
                              <span className={`status-badge status-${run.status === 'completed' ? 'pass' : run.status}`}>
                                {run.status.toUpperCase()}
                              </span>
                            </td>
                            <td>
                              <button
                                className="btn btn-sm"
                                onClick={() => exportRun(run.id)}
                                title="Download as JSON"
                                style={{ fontSize: 11, padding: '2px 8px' }}
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
              </>
            )}
          </div>
        )}

        {/* ── Pareto Tab ── */}
        {tab === 'pareto' && (
          <div className="bench-pareto">
            {paretoData.length === 0 ? (
              <div className="bench-empty">
                <p className="bench-empty-text">Run benchmarks with different models to see the Pareto frontier.</p>
              </div>
            ) : (
              <>
                <Pane title="PARETO FRONTIER">
                  <ParetoChart data={paretoData} height={400} />
                </Pane>

                <Pane title="MODEL COMPARISON">
                  <div className="task-table-wrap">
                    <table className="task-table">
                      <thead>
                        <tr>
                          <th>Model</th>
                          <th>Pass Rate</th>
                          <th>Cost</th>
                          <th>Tasks</th>
                          <th>Suite</th>
                        </tr>
                      </thead>
                      <tbody>
                        {history.filter((r) => r.summary).map((run) => (
                          <tr key={run.id}>
                            <td className="mono">{run.config.model}</td>
                            <td className="mono">{(run.summary!.pass_rate * 100).toFixed(1)}%</td>
                            <td className="mono">${run.summary!.total_cost_usd.toFixed(3)}</td>
                            <td className="mono">{run.summary!.total_tasks}</td>
                            <td>{run.suite_name}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </Pane>
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
