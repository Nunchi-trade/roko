import { useState, useEffect } from 'react';
import { Link } from 'react-router';
import { useBench } from '../hooks/useBench';
import type { AgentStrategy, BenchRun } from '../lib/bench-types';
import Pane from '../components/Pane';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import BarChart from '../components/Charts/BarChart';
import CostChart from '../components/Charts/CostChart';
import TimelineChart from '../components/Charts/TimelineChart';
import HeatmapChart from '../components/Charts/HeatmapChart';
import RadarChart from '../components/Charts/RadarChart';
import ScatterChart from '../components/Charts/ScatterChart';
import ModelPicker from '../components/ModelPicker';
import SuiteSelector from '../components/SuiteSelector';
import TaskTable from '../components/TaskTable';
import ConfigDiff from '../components/ConfigDiff';
import './Bench.css';

type Tab = 'configure' | 'live' | 'results' | 'history' | 'compare' | 'analysis';

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
  { id: 'compare', label: 'Compare' },
  { id: 'analysis', label: 'Analysis' },
];

const RUN_COLORS = [
  'var(--rose-bright)', 'var(--bone-bright)', 'var(--success)',
  'var(--dream-bright)', '#9B8EC4', '#C49B6E',
];

function formatEta(ms: number | null): string {
  if (!ms) return '';
  const s = Math.round(ms / 1000);
  if (s < 60) return `~${s}s`;
  return `~${Math.round(s / 60)}m ${s % 60}s`;
}

export default function Bench() {
  const [tab, setTab] = useState<Tab>('configure');
  const [selectedModel, setSelectedModel] = useState('claude-sonnet-4-20250514');
  const [selectedProvider, setSelectedProvider] = useState('anthropic');

  const bench = useBench();
  const {
    config, setConfig,
    selectedSuiteId, setSelectedSuiteId, selectedSuite,
    suites, models, history,
    suitesLoading, modelsLoading, historyLoading, connectionState,
    activeRun, activeRunSummary, feed, eta,
    startRun, cancelRun, exportRun, importRun,
    lastCompletedRun,
    compareIds, setCompareIds,
    pareto, fetchPareto,
  } = bench;

  // Fetch pareto data when analysis tab opens
  useEffect(() => {
    if (tab === 'analysis') fetchPareto();
  }, [tab, fetchPareto]);

  // Hero stats
  const totalRuns = history.length;
  const avgPassRate = history.length > 0
    ? history.reduce((s, r) => s + (r.summary?.pass_rate ?? 0), 0) / history.length
    : 0;
  const totalCost = history.reduce((s, r) => s + (r.summary?.total_cost_usd ?? 0), 0);

  // Results display
  const displayResults = activeRun?.results ?? (lastCompletedRun && 'results' in lastCompletedRun ? (lastCompletedRun as { results: typeof activeRun extends null ? never : NonNullable<typeof activeRun>['results'] }).results : []);
  const displaySummary = activeRunSummary ?? (lastCompletedRun && 'summary' in lastCompletedRun ? (lastCompletedRun as { summary?: NonNullable<typeof activeRunSummary> }).summary : undefined);

  // Compare tab: selected runs
  const compareRuns = history.filter((r) => compareIds.includes(r.id));

  // History multi-select
  const [historySelected, setHistorySelected] = useState<Set<string>>(new Set());
  const toggleHistorySelect = (id: string) => {
    setHistorySelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else if (next.size < 6) next.add(id);
      return next;
    });
  };

  // History filters
  const [historyFilter, setHistoryFilter] = useState({ suite: '', model: '', status: '' });
  const filteredHistory = history.filter((r) => {
    if (historyFilter.suite && r.suite_id !== historyFilter.suite) return false;
    if (historyFilter.model && r.config.model !== historyFilter.model) return false;
    if (historyFilter.status && r.status !== historyFilter.status) return false;
    return true;
  });

  // Cost estimate
  const selectedModelInfo = models.find((m) => m.id === selectedModel);
  const estimatedCost = selectedSuite && selectedModelInfo
    ? ((selectedModelInfo.cost_per_1k_input * 2 + selectedModelInfo.cost_per_1k_output * 3) * selectedSuite.tasks.length * 0.8)
    : null;

  return (
    <div className="bench-page">
      <div className="bench-hero">
        <div className="bench-hero-header">
          <h1 className="bench-page-title">Benchmark Lab</h1>
          <p className="bench-page-sub">
            Configure, run, and analyze agent evaluations
            {connectionState === 'offline' && <span className="bench-offline-badge">OFFLINE</span>}
          </p>
        </div>
        <div className="bench-hero-stats">
          <Mosaic columns={4}>
            <MosaicCell label="TOTAL RUNS" value={String(totalRuns || '-')} color="bone" />
            <MosaicCell label="AVG PASS RATE" value={avgPassRate > 0 ? `${(avgPassRate * 100).toFixed(0)}%` : '-'} color="success" />
            <MosaicCell label="TOTAL COST" value={totalCost > 0 ? `$${totalCost.toFixed(2)}` : '-'} color="warning" />
            <MosaicCell label="SUITES" value={suitesLoading ? '...' : String(suites.length)} color="rose" />
          </Mosaic>
        </div>
      </div>

      <div className="bench-tabs">
        {TABS.map((t) => (
          <button key={t.id} className={`bench-tab${tab === t.id ? ' active' : ''}`} onClick={() => setTab(t.id)}>
            {t.label}
            {t.id === 'live' && activeRun?.status === 'running' && <span className="bench-tab-dot" />}
          </button>
        ))}
      </div>

      <div className="bench-body">
        {/* ── Configure ── */}
        {tab === 'configure' && (
          <div className="bench-config-layout">
            <div className="bench-config-left">
              <Pane title="TEST SUITE">
                {suitesLoading ? <div className="bench-skeleton" style={{ height: 120 }} />
                  : suites.length === 0 ? <p className="bench-empty-text">No suites. Start roko serve.</p>
                  : <SuiteSelector suites={suites} value={selectedSuiteId} onChange={setSelectedSuiteId} />}
              </Pane>

              <Pane title="AGENT STRATEGY">
                <div className="config-cards">
                  {STRATEGIES.map((s) => (
                    <button key={s.id} className={`config-card${config.strategy === s.id ? ' selected' : ''}`}
                      onClick={() => setConfig({ ...config, strategy: s.id })}>
                      <span className="card-label">{s.label}</span>
                      <span className="card-desc">{s.desc}</span>
                    </button>
                  ))}
                </div>
              </Pane>

              <Pane title="MODEL">
                {modelsLoading ? <div className="bench-skeleton" style={{ height: 80 }} />
                  : models.length === 0 ? <p className="bench-empty-text">No models. Start roko serve.</p>
                  : <ModelPicker models={models} value={selectedModel}
                      onChange={(m, p) => { setSelectedModel(m); setSelectedProvider(p); }}
                      estimatedTasks={selectedSuite?.tasks.length} />}
              </Pane>
            </div>

            <div className="bench-config-right">
              <Pane title="PARAMETERS">
                <div className="config-params">
                  <label className="param-row">
                    <span className="param-label">Temperature</span>
                    <input type="range" min="0" max="1" step="0.1" value={config.temperature}
                      onChange={(e) => setConfig({ ...config, temperature: Number(e.target.value) })}
                      className="param-slider" />
                    <span className="param-value">{config.temperature}</span>
                  </label>
                  <label className="param-row">
                    <span className="param-label">Max Tokens</span>
                    <input type="number" className="config-input" value={config.maxTokens}
                      onChange={(e) => setConfig({ ...config, maxTokens: Number(e.target.value) })} style={{ maxWidth: 120 }} />
                  </label>
                  <label className="param-row">
                    <span className="param-label">Timeout (s)</span>
                    <input type="number" className="config-input" value={config.timeoutSecs}
                      onChange={(e) => setConfig({ ...config, timeoutSecs: Number(e.target.value) })} style={{ maxWidth: 120 }} />
                  </label>
                  <label className="param-row">
                    <span className="param-label">Retries</span>
                    <input type="number" className="config-input" min="0" max="3" value={config.retries}
                      onChange={(e) => setConfig({ ...config, retries: Number(e.target.value) })} style={{ maxWidth: 80 }} />
                  </label>
                  <div className="param-row">
                    <span className="param-label">Gates</span>
                    <div className="gate-toggles">
                      {(['compile', 'test', 'clippy', 'diff'] as const).map((g) => (
                        <label key={g} className="gate-toggle">
                          <input type="checkbox" checked={config.gates[g]}
                            onChange={(e) => setConfig({ ...config, gates: { ...config.gates, [g]: e.target.checked } })} />
                          <span>{g}</span>
                        </label>
                      ))}
                    </div>
                  </div>
                </div>
              </Pane>

              <Pane title="COST CALCULATOR">
                <div className="bench-cost-calc">
                  {selectedSuite && (
                    <>
                      <div className="cost-calc-row"><span className="cost-calc-label">Suite est.</span><span className="cost-calc-value">${selectedSuite.estimated_cost_usd.toFixed(2)}</span></div>
                      {estimatedCost != null && <div className="cost-calc-row"><span className="cost-calc-label">Model est.</span><span className="cost-calc-value">${estimatedCost.toFixed(3)}</span></div>}
                      <div className="cost-calc-row"><span className="cost-calc-label">Tasks</span><span className="cost-calc-value">{selectedSuite.tasks.length}</span></div>
                      <div className="cost-calc-row"><span className="cost-calc-label">Strategy</span><span className="cost-calc-value">{config.strategy.replace(/_/g, ' ')}</span></div>
                    </>
                  )}
                </div>
              </Pane>

              <div className="bench-run-btn">
                <button className="btn"
                  onClick={() => { startRun(selectedModel, selectedProvider); setTab('live'); }}
                  disabled={activeRun?.status === 'running' || connectionState === 'offline'}>
                  {activeRun?.status === 'running' ? 'Running...' : 'Run Benchmark'}
                </button>
              </div>
            </div>
          </div>
        )}

        {/* ── Live ── */}
        {tab === 'live' && (
          <div className="bench-live">
            {!activeRun ? (
              <div className="bench-empty--no-runs">
                <p className="bench-empty-text">No active run.</p>
                <button className="btn" onClick={() => setTab('configure')}>Start from Configure</button>
              </div>
            ) : (
              <>
                <div className="bench-live-header">
                  <span className={`benchlive-dot${activeRun.status === 'running' ? '' : ' disconnected'}`} />
                  <span className="bench-live-status">{activeRun.status === 'running' ? 'RUNNING' : activeRun.status.toUpperCase()}</span>
                  <span className="bench-live-progress">{activeRun.progress}/{activeRun.total}</span>
                  {eta != null && <span className="bench-live-eta">ETA {formatEta(eta)}</span>}
                  <span className="bench-live-cost">${activeRun.costSoFar.toFixed(3)}</span>
                  {activeRun.status === 'running' && (
                    <button className="btn btn-sm" onClick={cancelRun} style={{ marginLeft: 'auto' }}>Cancel</button>
                  )}
                </div>
                <div className="bench-live-progress-bar">
                  <div className="bench-live-progress-fill" style={{ width: `${activeRun.total > 0 ? (activeRun.progress / activeRun.total) * 100 : 0}%` }} />
                </div>

                {activeRun.results.length > 0 && (
                  <Pane title="TASK TIMELINE">
                    <TimelineChart
                      tasks={activeRun.results.map((r, i) => ({
                        name: r.task_name,
                        startMs: activeRun.results.slice(0, i).reduce((s, prev) => s + prev.duration_ms, 0),
                        durationMs: r.duration_ms,
                        status: r.status as 'pass' | 'fail' | 'pending' | 'running' | 'skipped',
                        gateVerdicts: r.gate_verdicts.map((g) => ({ gate: g.gate, passed: g.passed })),
                      }))}
                      height={Math.max(200, activeRun.results.length * 28 + 48)}
                    />
                  </Pane>
                )}

                <div className="benchlive-grid">
                  <Pane title="TASK GRID">
                    <div className="task-grid">
                      {Array.from({ length: activeRun.total }, (_, i) => {
                        const result = activeRun.results[i];
                        const status = result ? result.status : i === activeRun.results.length && activeRun.status === 'running' ? 'running' : 'pending';
                        return <div key={i} className={`task-cell task-${status}`} title={result ? result.task_name : `Task ${i + 1}`} />;
                      })}
                    </div>
                  </Pane>
                  <Pane title="COST CHART">
                    <CostChart data={activeRun.results.map((r, i) => ({ label: `T${i + 1}`, value: r.cost_usd }))} height={260} color="var(--bone)" />
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

        {/* ── Results ── */}
        {tab === 'results' && (
          <div className="bench-results">
            {displayResults.length === 0 ? (
              <div className="bench-empty--no-runs"><p className="bench-empty-text">No results yet. Run a benchmark first.</p></div>
            ) : (
              <>
                <div className="bench-results-stats">
                  <Mosaic columns={6}>
                    <MosaicCell label="PASS RATE" value={displaySummary ? `${(displaySummary.pass_rate * 100).toFixed(0)}%` : '-'} color="success" />
                    <MosaicCell label="TOTAL COST" value={displaySummary ? `$${displaySummary.total_cost_usd.toFixed(3)}` : '-'} color="warning" />
                    <MosaicCell label="USD/SUCCESS" value={displaySummary ? `$${displaySummary.cost_per_success_usd.toFixed(3)}` : '-'} color="bone" mono />
                    <MosaicCell label="AVG DURATION" value={displaySummary ? `${(displaySummary.avg_duration_ms / 1000).toFixed(1)}s` : '-'} color="dream" mono />
                    <MosaicCell label="TOTAL TOKENS" value={displaySummary ? displaySummary.total_tokens.toLocaleString() : '-'} color="rose" mono />
                    <MosaicCell label="TOKEN EFF." value={displaySummary && displaySummary.total_tokens > 0 ? `${(displaySummary.passed / (displaySummary.total_tokens / 1000)).toFixed(2)}/1K` : '-'} color="bone" mono />
                  </Mosaic>
                </div>

                <Pane title="TASK RESULTS"><TaskTable results={displayResults} /></Pane>

                <div className="bench-results-charts">
                  {displayResults.length > 0 && (() => {
                    const gateNames = [...new Set(displayResults.flatMap((r) => r.gate_verdicts.map((g) => g.gate)))];
                    const heatValues = displayResults.map((r) => gateNames.map((gate) => { const v = r.gate_verdicts.find((g) => g.gate === gate); return v ? v.passed : null; }));
                    return (
                      <Pane title="GATE PASS HEATMAP">
                        <HeatmapChart rows={displayResults.map((r) => r.task_name.slice(0, 20))} columns={gateNames} values={heatValues} height={Math.max(200, displayResults.length * 24 + 48)} />
                      </Pane>
                    );
                  })()}
                  <Pane title="COST PER TASK">
                    <BarChart data={displayResults.slice(-30).map((r) => ({ label: r.task_name.slice(0, 20), value: r.cost_usd, color: r.status === 'pass' ? 'var(--success)' : 'var(--rose-dim)' }))} height={250} />
                  </Pane>
                </div>

                {displayResults.some((r) => r.status === 'fail') && (
                  <Pane title="FAILED TASKS">
                    <div className="bench-failed-list">
                      {displayResults.filter((r) => r.status === 'fail').map((r) => (
                        <div key={r.task_id} className="bench-failed-item">
                          <div className="bench-failed-header">
                            <span className="task-name">{r.task_name}</span>
                            <span className="mono gate-err">${r.cost_usd.toFixed(3)}</span>
                          </div>
                          {r.error && <div className="task-error">{r.error}</div>}
                          {r.output_preview && <pre className="task-output-code">{r.output_preview}</pre>}
                        </div>
                      ))}
                    </div>
                  </Pane>
                )}
              </>
            )}
          </div>
        )}

        {/* ── History ── */}
        {tab === 'history' && (
          <div className="bench-history">
            <div className="bench-history-toolbar">
              <div className="bench-history-filters">
                <select className="config-input" style={{ maxWidth: 160 }} value={historyFilter.suite} onChange={(e) => setHistoryFilter({ ...historyFilter, suite: e.target.value })}>
                  <option value="">All suites</option>
                  {[...new Set(history.map((r) => r.suite_id))].map((sid) => <option key={sid} value={sid}>{history.find((r) => r.suite_id === sid)?.suite_name ?? sid}</option>)}
                </select>
                <select className="config-input" style={{ maxWidth: 160 }} value={historyFilter.model} onChange={(e) => setHistoryFilter({ ...historyFilter, model: e.target.value })}>
                  <option value="">All models</option>
                  {[...new Set(history.map((r) => r.config.model))].map((m) => <option key={m} value={m}>{m.split('-').slice(0, 2).join('-')}</option>)}
                </select>
              </div>
              <div className="bench-history-actions">
                {historySelected.size >= 2 && (
                  <button className="btn btn-sm" onClick={() => { setCompareIds([...historySelected]); setTab('compare'); }}>Compare ({historySelected.size})</button>
                )}
                <label className="btn btn-sm" style={{ cursor: 'pointer' }}>Import<input type="file" accept=".json" style={{ display: 'none' }} onChange={(e) => { const file = e.target.files?.[0]; if (file) importRun(file); e.target.value = ''; }} /></label>
              </div>
            </div>

            {historyLoading ? <div className="bench-skeleton" style={{ height: 200 }} />
              : filteredHistory.length === 0 ? <div className="bench-empty--no-runs"><p className="bench-empty-text">No runs recorded yet.</p></div>
              : (
                <Pane title={`RUN HISTORY (${filteredHistory.length})`}>
                  <div className="task-table-wrap">
                    <table className="task-table">
                      <thead><tr><th style={{ width: 32 }}></th><th>Date</th><th>Suite</th><th>Model</th><th>Strategy</th><th>Pass Rate</th><th>Cost</th><th>Duration</th><th>Status</th><th>Actions</th></tr></thead>
                      <tbody>
                        {filteredHistory.map((run) => (
                          <tr key={run.id} className={historySelected.has(run.id) ? 'diff-changed' : ''}>
                            <td><input type="checkbox" checked={historySelected.has(run.id)} onChange={() => toggleHistorySelect(run.id)} style={{ accentColor: 'var(--rose-bright)' }} /></td>
                            <td className="mono">{new Date(run.started_at).toLocaleDateString()}</td>
                            <td>{run.suite_name}</td>
                            <td className="mono">{run.config.model.split('-').slice(0, 2).join('-')}</td>
                            <td>{run.config.strategy.replace(/_/g, ' ')}</td>
                            <td className="mono">{run.summary ? `${(run.summary.pass_rate * 100).toFixed(0)}%` : '-'}</td>
                            <td className="mono">{run.summary ? `$${run.summary.total_cost_usd.toFixed(3)}` : '-'}</td>
                            <td className="mono">{run.summary ? `${(run.summary.total_duration_ms / 1000).toFixed(1)}s` : '-'}</td>
                            <td><span className={`status-badge status-${run.status === 'completed' ? 'pass' : run.status}`}>{run.status.toUpperCase()}</span></td>
                            <td style={{ display: 'flex', gap: 4 }}>
                              <Link to={`/bench/run/${run.id}`} className="btn btn-sm" style={{ textDecoration: 'none', fontSize: 10, padding: '2px 6px' }}>View</Link>
                              <button className="btn btn-sm" onClick={() => exportRun(run.id)} style={{ fontSize: 10, padding: '2px 6px' }}>Export</button>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </Pane>
              )}
          </div>
        )}

        {/* ── Compare ── */}
        {tab === 'compare' && (
          <div className="bench-compare">
            <Pane title="SELECT RUNS TO COMPARE">
              <div className="bench-compare-chips">
                {compareIds.map((id) => {
                  const run = history.find((r) => r.id === id);
                  return (
                    <div key={id} className="bench-chip">
                      <span>{run ? `${run.id.slice(0, 8)} · ${run.suite_name}` : id.slice(0, 8)}</span>
                      <button className="bench-chip-x" onClick={() => setCompareIds(compareIds.filter((x) => x !== id))}>&times;</button>
                    </div>
                  );
                })}
                {compareIds.length < 6 && (
                  <select className="config-input" style={{ maxWidth: 200 }} value="" onChange={(e) => {
                    if (e.target.value && !compareIds.includes(e.target.value)) setCompareIds([...compareIds, e.target.value]);
                  }}>
                    <option value="">Add run...</option>
                    {history.filter((r) => !compareIds.includes(r.id)).map((r) => (
                      <option key={r.id} value={r.id}>{r.id.slice(0, 8)} - {r.suite_name} ({r.config.model.split('-').slice(0, 2).join('-')})</option>
                    ))}
                  </select>
                )}
              </div>
              <div className="bench-compare-quick">
                <button className="btn btn-sm" onClick={() => { if (history.length >= 2) setCompareIds(history.slice(0, 2).map((r) => r.id)); }}>Last 2</button>
                <button className="btn btn-sm" onClick={() => { const sid = history[0]?.suite_id; if (sid) setCompareIds(history.filter((r) => r.suite_id === sid).slice(0, 4).map((r) => r.id)); }}>Same Suite</button>
                <button className="btn btn-sm" onClick={() => { const m = history[0]?.config.model; if (m) setCompareIds(history.filter((r) => r.config.model === m).slice(0, 4).map((r) => r.id)); }}>Same Model</button>
              </div>
            </Pane>

            {compareRuns.length >= 2 ? (
              <>
                <Pane title="CONFIG DIFF"><ConfigDiff runs={compareRuns} /></Pane>

                {(() => {
                  const axes = ['Pass Rate', 'Speed', 'Cost Eff.', 'Token Eff.', 'Gate Pass'];
                  const datasets = compareRuns.map((run, i) => {
                    const s = run.summary;
                    if (!s) return null;
                    let gp = 0, gt = 0;
                    for (const res of run.results) for (const g of res.gate_verdicts) { gt++; if (g.passed) gp++; }
                    return {
                      label: run.config.model.split('-').slice(0, 2).join('-'),
                      values: [s.pass_rate, 1 - Math.min(s.avg_duration_ms / 60000, 1), 1 - Math.min(s.total_cost_usd / 1, 1), Math.min(s.passed / Math.max(s.total_tokens / 1000, 0.001), 1), gt > 0 ? gp / gt : 0],
                      color: RUN_COLORS[i % RUN_COLORS.length],
                    };
                  }).filter((d): d is NonNullable<typeof d> => d != null);
                  return datasets.length >= 2 ? <Pane title="RADAR COMPARISON"><RadarChart axes={axes} datasets={datasets} height={350} /></Pane> : null;
                })()}
              </>
            ) : (
              <div className="bench-empty--no-runs"><p className="bench-empty-text">Select at least 2 runs to compare.</p></div>
            )}
          </div>
        )}

        {/* ── Analysis ── */}
        {tab === 'analysis' && (
          <div className="bench-analysis">
            <Pane title="PARETO FRONTIER">
              {(() => {
                const pts = pareto?.points ?? [];
                const histPts = history.filter((r) => r.summary);
                const scatterData = pts.length > 0
                  ? pts.map((p) => ({ x: p.cost_usd, y: p.pass_rate, label: p.label ?? p.run_id.slice(0, 8), color: p.provider?.includes('Anthropic') ? 'var(--rose-bright)' : 'var(--success)' }))
                  : histPts.map((r) => ({ x: r.summary!.total_cost_usd, y: r.summary!.pass_rate, label: r.config.model.split('-').slice(0, 2).join('-'), color: r.config.model.includes('sonnet') ? 'var(--rose-bright)' : 'var(--success)' }));
                return scatterData.length > 0
                  ? <ScatterChart points={scatterData} xLabel="Cost (USD)" yLabel="Pass Rate" showTrendLine height={400} />
                  : <p className="bench-empty-text">Run benchmarks to see the Pareto frontier.</p>;
              })()}
            </Pane>

            {history.filter((r) => r.summary).length > 0 && (
              <>
                <Pane title="MODEL LEADERBOARD">
                  <div className="task-table-wrap">
                    <table className="task-table">
                      <thead><tr><th>#</th><th>Model</th><th>Runs</th><th>Avg Pass Rate</th><th>Avg Cost</th><th>Avg Duration</th></tr></thead>
                      <tbody>
                        {(() => {
                          const byModel = new Map<string, BenchRun[]>();
                          for (const r of history.filter((r) => r.summary)) { const l = byModel.get(r.config.model) ?? []; l.push(r); byModel.set(r.config.model, l); }
                          return [...byModel.entries()].map(([model, runs]) => ({
                            model, runs: runs.length,
                            avgPass: runs.reduce((s, r) => s + (r.summary?.pass_rate ?? 0), 0) / runs.length,
                            avgCost: runs.reduce((s, r) => s + (r.summary?.total_cost_usd ?? 0), 0) / runs.length,
                            avgDur: runs.reduce((s, r) => s + (r.summary?.total_duration_ms ?? 0), 0) / runs.length,
                          })).sort((a, b) => b.avgPass - a.avgPass || a.avgCost - b.avgCost).map((row, i) => (
                            <tr key={row.model}><td className="mono">{i + 1}</td><td className="mono">{row.model}</td><td className="mono">{row.runs}</td><td className="mono">{(row.avgPass * 100).toFixed(1)}%</td><td className="mono">${row.avgCost.toFixed(3)}</td><td className="mono">{(row.avgDur / 1000).toFixed(1)}s</td></tr>
                          ));
                        })()}
                      </tbody>
                    </table>
                  </div>
                </Pane>

                <Pane title="STRATEGY LEADERBOARD">
                  <div className="task-table-wrap">
                    <table className="task-table">
                      <thead><tr><th>#</th><th>Strategy</th><th>Runs</th><th>Avg Pass Rate</th><th>Avg Cost</th></tr></thead>
                      <tbody>
                        {(() => {
                          const byStrat = new Map<string, BenchRun[]>();
                          for (const r of history.filter((r) => r.summary)) { const l = byStrat.get(r.config.strategy) ?? []; l.push(r); byStrat.set(r.config.strategy, l); }
                          return [...byStrat.entries()].map(([strat, runs]) => ({
                            strat, runs: runs.length,
                            avgPass: runs.reduce((s, r) => s + (r.summary?.pass_rate ?? 0), 0) / runs.length,
                            avgCost: runs.reduce((s, r) => s + (r.summary?.total_cost_usd ?? 0), 0) / runs.length,
                          })).sort((a, b) => b.avgPass - a.avgPass).map((row, i) => (
                            <tr key={row.strat}><td className="mono">{i + 1}</td><td>{row.strat.replace(/_/g, ' ')}</td><td className="mono">{row.runs}</td><td className="mono">{(row.avgPass * 100).toFixed(1)}%</td><td className="mono">${row.avgCost.toFixed(3)}</td></tr>
                          ));
                        })()}
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
