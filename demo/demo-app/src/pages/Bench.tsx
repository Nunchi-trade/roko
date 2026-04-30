import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { Link } from 'react-router';
import { useBench } from '../hooks/useBench';
import { handleRowKeyDown } from '../lib/a11y';
import { useRokoConfig } from '../hooks/useRokoConfig';
import { useToast } from '../components/Toast';
import type { AgentStrategy, BenchRun } from '../lib/bench-types';
import Pane from '../components/Pane';
import Tooltip from '../components/Tooltip';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import BarChart from '../components/Charts/BarChart';
import CostChart from '../components/Charts/CostChart';
import TimelineChart from '../components/Charts/TimelineChart';
import HeatmapChart from '../components/Charts/HeatmapChart';
import RadarChart from '../components/Charts/RadarChart';
import ScatterChart from '../components/Charts/ScatterChart';
import SuiteSelector from '../components/SuiteSelector';
import TaskTable from '../components/TaskTable';
import ConfigDiff from '../components/ConfigDiff';
import CostRace from '../components/CostRace';
import AgentOutputStream from '../components/AgentOutputStream';
import GateVerdictTicker from '../components/GateVerdictTicker';
import TokenVelocitySparkline from '../components/TokenVelocitySparkline';
import BenchLearningInsights from '../components/BenchLearningInsights';
import MatrixBuilder from '../components/MatrixBuilder';
import MatrixRaceTrack from '../components/MatrixRaceTrack';
import MatrixDetailView from '../components/MatrixDetailView';
import { ComponentErrorBoundary, DataSurface } from '../components/design';
import { Sparkle } from '../components/Celebration';
import { PulseIcon, SpinnerIcon, CheckmarkIcon, CrossIcon } from '../components/icons/AnimatedIcons';
import { useMatrixBench } from '../hooks/useMatrixBench';
import { useCountUp, fmtCount, fmtCost } from '../hooks/useCountUp';
import './Bench.css';
import '../styles/bench-compare.css';
import '../styles/bench-race.css';

/* ── Animated number counter ── */
function AnimatedNumber({ value, prefix = '', suffix = '', decimals = 0 }: {
  value: number; prefix?: string; suffix?: string; decimals?: number;
}) {
  const [displayed, setDisplayed] = useState(value);
  const rafRef = useRef(0);
  const prevRef = useRef(value);
  const prefersReducedMotion = useMemo(
    () => typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches,
    [],
  );

  useEffect(() => {
    if (prefersReducedMotion) { setDisplayed(value); return; }
    const from = prevRef.current;
    prevRef.current = value;
    if (from === value) return;
    const start = performance.now();
    const duration = 500;
    function tick(now: number) {
      const t = Math.min((now - start) / duration, 1);
      const eased = 1 - Math.pow(1 - t, 3); // easeOutCubic
      setDisplayed(from + (value - from) * eased);
      if (t < 1) rafRef.current = requestAnimationFrame(tick);
    }
    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
  }, [value, prefersReducedMotion]);

  return <>{prefix}{displayed.toFixed(decimals)}{suffix}</>;
}

/* ── Countdown overlay ── */
function CountdownOverlay({ onDone }: { onDone: () => void }) {
  const [count, setCount] = useState(3);

  useEffect(() => {
    if (count <= 0) { onDone(); return; }
    const timer = setTimeout(() => setCount(count - 1), 800);
    return () => clearTimeout(timer);
  }, [count, onDone]);

  if (count <= 0) return null;

  return (
    <div className="bench-countdown-overlay">
      <span key={count} className="bench-countdown-number">{count}</span>
    </div>
  );
}

type Tab = 'configure' | 'live' | 'results' | 'history' | 'compare' | 'analysis' | 'learning';

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
  { id: 'learning', label: 'Learning' },
];

const RUN_COLORS = [
  'var(--rose-bright)', 'var(--bone-bright)', 'var(--success)',
  'var(--dream-bright)', 'var(--dream)', 'var(--warning)',
];

function formatEta(ms: number | null): string {
  if (!ms) return '';
  const s = Math.round(ms / 1000);
  if (s < 60) return `~${s}s`;
  return `~${Math.round(s / 60)}m ${s % 60}s`;
}

type ConfigureMode = 'single' | 'matrix';
type LiveViewMode = 'race' | 'detail';

export default function Bench() {
  const [tab, setTab] = useState<Tab>('configure');
  const [configureMode, setConfigureMode] = useState<ConfigureMode>('single');
  const [liveViewMode, setLiveViewMode] = useState<LiveViewMode>('race');
  const [showCountdown, setShowCountdown] = useState(false);
  const [runCompleted, setRunCompleted] = useState(false);
  const pendingStartRef = useRef<(() => void) | null>(null);
  const { defaultModel, defaultBackend } = useRokoConfig();
  const { toast } = useToast();

  const bench = useBench();
  const {
    config, setConfig,
    selectedSuiteId, setSelectedSuiteId, selectedSuite,
    suites, models, history,
    suitesLoading, historyLoading, connectionState,
    activeRun, activeRunSummary, activeRunLearning, feed, eta,
    agentOutput, currentAgentId, gateVerdicts, tokenVelocity,
    startRun, cancelRun, exportRun, importRun,
    compareIds, setCompareIds,
    pareto, fetchPareto,
  } = bench;

  // Matrix mode
  const matrix = useMatrixBench(models);

  // Countdown callback: fire the actual start after countdown finishes
  const handleCountdownDone = useCallback(() => {
    setShowCountdown(false);
    pendingStartRef.current?.();
    pendingStartRef.current = null;
  }, []);

  // Detect run completion for burst effect
  const prevRunStatus = useRef(activeRun?.status);
  useEffect(() => {
    if (prevRunStatus.current === 'running' && activeRun?.status === 'completed') {
      setRunCompleted(true);
      const passRate = activeRunSummary?.pass_rate;
      toast(
        passRate != null
          ? `Benchmark complete: ${(passRate * 100).toFixed(0)}% pass rate`
          : 'Benchmark complete',
        { type: passRate != null && passRate >= 0.5 ? 'success' : 'warning' },
      );
      const timer = setTimeout(() => setRunCompleted(false), 1200);
      return () => clearTimeout(timer);
    }
    prevRunStatus.current = activeRun?.status;
  }, [activeRun?.status, activeRunSummary, toast]);

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

  const animRuns = useCountUp(totalRuns);
  const animPassRate = useCountUp(avgPassRate * 100);
  const animCost = useCountUp(totalCost);

  // Results display: prefer active run, fall back to last completed from history
  const lastHistoryRun = history.find((r) => r.status === 'completed');
  const displayResults = activeRun?.results ?? lastHistoryRun?.results ?? [];
  const displaySummary = activeRunSummary ?? lastHistoryRun?.summary;

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
  const selectedModelInfo = models.find((m) => m.id === defaultModel);
  const estimatedCost = selectedSuite && selectedModelInfo
    ? ((selectedModelInfo.cost_per_1k_input * 2 + selectedModelInfo.cost_per_1k_output * 3) * selectedSuite.tasks.length * 0.8)
    : null;

  return (
    <div className="bench-page">
      {showCountdown && <CountdownOverlay onDone={handleCountdownDone} />}
      <div className="bench-hero">
        <div className="bench-hero-header">
          <h1 className="bench-page-title text-gradient-cool text-glow">Benchmark Lab</h1>
          <p className="bench-page-sub">
            Configure, run, and analyze agent evaluations
            {connectionState === 'offline' && <span className="bench-offline-badge">OFFLINE</span>}
          </p>
        </div>
        <div className="bench-hero-stats">
          <Mosaic columns={4}>
            <Tooltip content="Total benchmark runs executed" placement="bottom"><MosaicCell label="TOTAL RUNS" value={<span className="counter-animate">{totalRuns > 0 ? fmtCount(animRuns) : '-'}</span>} color="bone" /></Tooltip>
            <Tooltip content="Average task pass rate across all runs" placement="bottom"><MosaicCell label="AVG PASS RATE" value={<span className="counter-animate">{avgPassRate > 0 ? `${fmtCount(animPassRate)}%` : '-'}</span>} color="success" /></Tooltip>
            <Tooltip content="Cumulative LLM spend across all runs" placement="bottom"><MosaicCell label="TOTAL COST" value={<span className="counter-animate">{totalCost > 0 ? fmtCost(animCost) : '-'}</span>} color="warning" /></Tooltip>
            <Tooltip content="Available test suites from roko serve" placement="bottom"><MosaicCell label="SUITES" value={suitesLoading ? '...' : String(suites.length)} color="rose" /></Tooltip>
          </Mosaic>
        </div>
      </div>

      <div className="bench-tabs">
        {TABS.map((t) => (
          <button key={t.id} className={`bench-tab btn-ghost-reveal${tab === t.id ? ' active' : ''}`} onClick={() => setTab(t.id)}>
            {t.label}
            {t.id === 'live' && activeRun?.status === 'running' && <span className="bench-tab-dot" />}
            {t.id === 'learning' && activeRun?.status === 'running' && <span className="bench-tab-dot" style={{ background: 'var(--dream-bright)' }} />}
          </button>
        ))}
      </div>

      <DataSurface
        loading={suitesLoading && historyLoading}
        empty={connectionState === 'offline' && suites.length === 0 && history.length === 0}
        emptyLabel="Server offline. Start roko serve to use benchmarks."
      >
      <div className="bench-body">
        {/* ── Configure ── */}
        {tab === 'configure' && (
          <>
            <div className="bench-mode-toggle">
              <button className={`bench-mode-btn btn-ghost-reveal${configureMode === 'single' ? ' active' : ''}`} onClick={() => setConfigureMode('single')}>Single</button>
              <button className={`bench-mode-btn btn-ghost-reveal${configureMode === 'matrix' ? ' active' : ''}`} onClick={() => setConfigureMode('matrix')}>Matrix</button>
            </div>

            {/* Shared suite selector */}
            <Pane title="TEST SUITE">
              {suitesLoading ? <div className="bench-skeleton skeleton" style={{ height: 120 }} />
                : suites.length === 0 ? <p className="bench-empty-text">No suites. Start roko serve.</p>
                : <SuiteSelector suites={suites} value={selectedSuiteId} onChange={setSelectedSuiteId} />}
            </Pane>

            {configureMode === 'single' ? (
              <div className="bench-config-layout">
                <div className="bench-config-left">
                  <Pane title="AGENT STRATEGY">
                    <div className="config-cards">
                      {STRATEGIES.map((s) => (
                        <button key={s.id} className={`config-card btn-interactive${config.strategy === s.id ? ' selected' : ''}`}
                          onClick={() => setConfig({ ...config, strategy: s.id })}>
                          <span className="card-label">{s.label}</span>
                          <span className="card-desc">{s.desc}</span>
                        </button>
                      ))}
                    </div>
                  </Pane>

                  <Pane title="MODEL">
                    <div className="bench-model-display">
                      <div className="param-row">
                        <span className="param-label">Model</span>
                        <span className="param-value mono">{defaultModel || '--'}</span>
                      </div>
                      <div className="param-row">
                        <span className="param-label">Backend</span>
                        <span className="param-value mono">{defaultBackend || '--'}</span>
                      </div>
                      <p className="bench-model-hint">Change via config pill</p>
                    </div>
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
                        <input type="number" className="config-input input-focus-glow" value={config.maxTokens}
                          onChange={(e) => setConfig({ ...config, maxTokens: Number(e.target.value) })} style={{ maxWidth: 120 }} />
                      </label>
                      <label className="param-row">
                        <span className="param-label">Timeout (s)</span>
                        <input type="number" className="config-input input-focus-glow" value={config.timeoutSecs}
                          onChange={(e) => setConfig({ ...config, timeoutSecs: Number(e.target.value) })} style={{ maxWidth: 120 }} />
                      </label>
                      <label className="param-row">
                        <span className="param-label">Retries</span>
                        <input type="number" className="config-input input-focus-glow" min="0" max="3" value={config.retries}
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
                    <button className="btn btn-primary-glow"
                      onClick={() => {
                        pendingStartRef.current = () => { startRun(defaultModel, defaultBackend); };
                        setShowCountdown(true);
                        setTab('live');
                      }}
                      disabled={activeRun?.status === 'running' || connectionState === 'offline'}>
                      {activeRun?.status === 'running' ? 'Running...' : 'Run Benchmark'}
                    </button>
                  </div>
                </div>
              </div>
            ) : (
              <ComponentErrorBoundary name="MatrixBuilder">
                <Pane title="MATRIX BUILDER">
                  <MatrixBuilder
                    models={models}
                    selectedModels={matrix.selectedModels}
                    toggleModel={matrix.toggleModel}
                    presets={matrix.presets}
                    togglePreset={matrix.togglePreset}
                    cells={matrix.cells}
                    totalLanes={matrix.totalLanes}
                    matrixStatus={matrix.status}
                    estimatedCostPerLane={selectedSuite?.estimated_cost_usd ?? 0}
                    onLaunch={() => {
                      if (selectedSuiteId) {
                        matrix.startMatrix(selectedSuiteId, {
                          temperature: config.temperature,
                          max_tokens: config.maxTokens,
                          timeout_secs: config.timeoutSecs,
                          retries: config.retries,
                          gates: config.gates,
                        });
                        setTab('live');
                      }
                    }}
                    disabled={connectionState === 'offline'}
                  />
                </Pane>
              </ComponentErrorBoundary>
            )}
          </>
        )}

        {/* ── Live ── */}
        {tab === 'live' && (
          <div className="bench-live">
            {matrix.status !== 'idle' ? (
              /* Matrix live view */
              <>
                <div className="bench-live-header">
                  {matrix.status === 'running'
                    ? <PulseIcon size={10} color="var(--success)" />
                    : <span className="benchlive-dot disconnected" />}
                  <span className="bench-live-status">MATRIX {matrix.status.toUpperCase()}</span>
                  <span className="bench-live-progress">{matrix.totalLanes} lanes</span>
                  <div className="bench-mode-toggle" style={{ marginLeft: 'auto', marginBottom: 0 }}>
                    <button className={`bench-mode-btn btn-ghost-reveal${liveViewMode === 'race' ? ' active' : ''}`} onClick={() => setLiveViewMode('race')}>Race</button>
                    <button className={`bench-mode-btn btn-ghost-reveal${liveViewMode === 'detail' ? ' active' : ''}`} onClick={() => setLiveViewMode('detail')}>Detail</button>
                  </div>
                </div>

                {liveViewMode === 'race' ? (
                  <ComponentErrorBoundary name="MatrixRaceTrack">
                    <Pane title="MATRIX RACE TRACK">
                      <MatrixRaceTrack
                        cells={matrix.cells}
                        selectedModels={matrix.selectedModels}
                        presetLabels={matrix.presets.map((p) => p.label)}
                        totalTasksPerLane={selectedSuite?.tasks.length ?? 0}
                      />
                    </Pane>
                  </ComponentErrorBoundary>
                ) : (
                  <ComponentErrorBoundary name="MatrixDetailView">
                    <Pane title="MATRIX DETAIL">
                      <MatrixDetailView
                        cells={matrix.cells}
                        selectedModels={matrix.selectedModels}
                        presetLabels={matrix.presets.map((p) => p.label)}
                        tasks={selectedSuite?.tasks ?? []}
                      />
                    </Pane>
                  </ComponentErrorBoundary>
                )}

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
              </>
            ) : !activeRun ? (
              <div className="bench-empty--no-runs">
                <p className="bench-empty-text">No active run.</p>
                <button className="btn" onClick={() => setTab('configure')}>Start from Configure</button>
              </div>
            ) : (
              /* Single run live view (unchanged) */
              <>
                <div className={`bench-live-header${activeRun.status === 'running' ? ' gradient-border-active' : ''}`} style={{ position: 'relative' }}>
                  <Sparkle active={runCompleted} count={7} duration={1000} onDone={() => setRunCompleted(false)} />
                  {activeRun.status === 'running'
                    ? <PulseIcon size={10} color="var(--success)" />
                    : activeRun.status === 'completed'
                      ? <CheckmarkIcon size={14} color="var(--success)" />
                      : activeRun.status === 'failed'
                        ? <CrossIcon size={14} color="var(--rose-bright)" />
                        : <SpinnerIcon size={10} />}
                  <span className={`bench-live-status${runCompleted ? ' completed-burst' : ''}`}>{activeRun.status === 'running' ? 'RUNNING' : activeRun.status.toUpperCase()}</span>
                  <span className="bench-live-progress">
                    {activeRun.progress}/{activeRun.total}
                    {activeRun.results.length > 0 && (
                      <span className="bench-live-breakdown">
                        {' '}(<span style={{ color: 'var(--success)' }}>{activeRun.results.filter(r => r.status === 'pass').length}P</span>
                        {' / '}
                        <span style={{ color: 'var(--rose-bright)' }}>{activeRun.results.filter(r => r.status === 'fail').length}F</span>)
                      </span>
                    )}
                  </span>
                  {eta != null && <span className="bench-live-eta">ETA {formatEta(eta)}</span>}
                  <span className="bench-live-cost bench-score-counter">
                    <AnimatedNumber value={activeRun.costSoFar} prefix="$" decimals={3} />
                  </span>
                  {activeRun.status === 'running' && (
                    <button className="btn btn-sm" onClick={cancelRun} style={{ marginLeft: 'auto' }}>Cancel</button>
                  )}
                </div>
                {activeRun.status !== 'running' && activeRun.results.length > 0 && activeRun.results.every(r => r.status === 'fail') && (
                  <div className="bench-live-error-banner">
                    All tasks failed. {activeRun.results[0]?.error
                      ? <>First error: <code>{activeRun.results[0].error.slice(0, 200)}</code></>
                      : 'Check that the configured model/provider has valid API credentials.'}
                  </div>
                )}
                <div className="bench-live-progress-bar">
                  <div className="bench-live-progress-fill" style={{ width: `${activeRun.total > 0 ? (activeRun.progress / activeRun.total) * 100 : 0}%` }} />
                </div>

                {activeRun.results.length > 0 && (
                  <ComponentErrorBoundary name="TaskTimeline">
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
                  </ComponentErrorBoundary>
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

                <div className="bench-live-visualizations">
                  <ComponentErrorBoundary name="AgentOutputStream">
                    <Pane title="AGENT OUTPUT">
                      <AgentOutputStream lines={agentOutput} agentId={currentAgentId} />
                    </Pane>
                  </ComponentErrorBoundary>
                  <ComponentErrorBoundary name="GateVerdictTicker">
                    <Pane title="GATE VERDICTS">
                      <GateVerdictTicker
                        verdicts={gateVerdicts}
                        currentTaskId={activeRun.results.length < activeRun.total
                          ? feed.find((f) => f.type === 'start')?.text.replace('Started: ', '')
                          : undefined}
                      />
                    </Pane>
                  </ComponentErrorBoundary>
                  <ComponentErrorBoundary name="TokenVelocitySparkline">
                    <Pane title="TOKEN VELOCITY">
                      <TokenVelocitySparkline points={tokenVelocity} height={120} />
                    </Pane>
                  </ComponentErrorBoundary>
                </div>

                <ComponentErrorBoundary name="CostRace">
                  <Pane title="COST RACE (LIVE)">
                    <CostRace live height={260} />
                  </Pane>
                </ComponentErrorBoundary>
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
                      <ComponentErrorBoundary name="GateHeatmap">
                        <Pane title="GATE PASS HEATMAP">
                          <HeatmapChart rows={displayResults.map((r) => r.task_name.slice(0, 20))} columns={gateNames} values={heatValues} height={Math.max(200, displayResults.length * 24 + 48)} />
                        </Pane>
                      </ComponentErrorBoundary>
                    );
                  })()}
                  <ComponentErrorBoundary name="CostPerTask">
                    <Pane title="COST PER TASK">
                      <BarChart data={displayResults.slice(-30).map((r) => ({ label: r.task_name.slice(0, 20), value: r.cost_usd, color: r.status === 'pass' ? 'var(--success)' : 'var(--rose-dim)' }))} height={250} />
                    </Pane>
                  </ComponentErrorBoundary>
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
                <select className="config-input input-focus-glow" style={{ maxWidth: 160 }} value={historyFilter.suite} onChange={(e) => setHistoryFilter({ ...historyFilter, suite: e.target.value })}>
                  <option value="">All suites</option>
                  {[...new Set(history.map((r) => r.suite_id))].map((sid) => <option key={sid} value={sid}>{history.find((r) => r.suite_id === sid)?.suite_name ?? sid}</option>)}
                </select>
                <select className="config-input input-focus-glow" style={{ maxWidth: 160 }} value={historyFilter.model} onChange={(e) => setHistoryFilter({ ...historyFilter, model: e.target.value })}>
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

            {historyLoading ? <div className="bench-skeleton skeleton" style={{ height: 200 }} />
              : filteredHistory.length === 0 ? <div className="bench-empty--no-runs"><p className="bench-empty-text">No runs recorded yet.</p></div>
              : (
                <Pane title={`RUN HISTORY (${filteredHistory.length})`}>
                  <div className="task-table-wrap">
                    <table className="task-table">
                      <thead><tr><th style={{ width: 32 }}></th><th>Date</th><th>Suite</th><th>Model</th><th>Strategy</th><th>Pass Rate</th><th>Cost</th><th>Duration</th><th>Status</th><th>Actions</th></tr></thead>
                      <tbody>
                        {filteredHistory.map((run) => (
                          <tr key={run.id} tabIndex={0} role="row" className={historySelected.has(run.id) ? 'diff-changed' : ''} onKeyDown={(e) => handleRowKeyDown(e, () => toggleHistorySelect(run.id))}>
                            <td><input type="checkbox" checked={historySelected.has(run.id)} onChange={() => toggleHistorySelect(run.id)} style={{ accentColor: 'var(--rose-bright)' }} /></td>
                            <td className="mono">{new Date(run.started_at).toLocaleDateString()}</td>
                            <td>{run.suite_name}</td>
                            <td className="mono">{run.config.model.split('-').slice(0, 2).join('-')}</td>
                            <td>{run.config.strategy.replace(/_/g, ' ')}</td>
                            <td className="mono">{run.summary ? `${(run.summary.pass_rate * 100).toFixed(0)}%` : '-'}</td>
                            <td className="mono">{run.summary ? `$${run.summary.total_cost_usd.toFixed(3)}` : '-'}</td>
                            <td className="mono">{run.summary ? `${(run.summary.total_duration_ms / 1000).toFixed(1)}s` : '-'}</td>
                            <td>
                              <span className={`status-badge status-${run.status === 'completed' ? 'pass' : run.status}`}>
                                {run.status === 'completed' ? <CheckmarkIcon size={12} color="var(--success)" /> : run.status === 'failed' ? <CrossIcon size={12} color="var(--rose-bright)" /> : run.status === 'running' ? <SpinnerIcon size={12} /> : null}
                                {' '}{run.status.toUpperCase()}
                              </span>
                            </td>
                            <td style={{ display: 'flex', gap: 'var(--sp-1)' }}>
                              <Link to={`/bench/run/${run.id}`} className="btn btn-sm" style={{ textDecoration: 'none', fontSize: 'var(--text-sm)', padding: '2px var(--sp-1)' }}>View</Link>
                              <button className="btn btn-sm" onClick={() => exportRun(run.id)} style={{ fontSize: 'var(--text-sm)', padding: '2px var(--sp-1)' }}>Export</button>
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
                    <div key={id} className="bench-chip chip-interactive">
                      <span>{run ? `${run.id.slice(0, 8)} · ${run.suite_name}` : id.slice(0, 8)}</span>
                      <button className="bench-chip-x" onClick={() => setCompareIds(compareIds.filter((x) => x !== id))}>&times;</button>
                    </div>
                  );
                })}
                {compareIds.length < 6 && (
                  <select className="config-input input-focus-glow" style={{ maxWidth: 200 }} value="" onChange={(e) => {
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
            <ComponentErrorBoundary name="ParetoFrontier">
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
            </ComponentErrorBoundary>

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

            <ComponentErrorBoundary name="ModelCostRace">
              <Pane title="MODEL COST RACE">
                <CostRace height={300} />
              </Pane>
            </ComponentErrorBoundary>
          </div>
        )}

        {/* ── Learning ── */}
        {tab === 'learning' && (
          <ComponentErrorBoundary name="BenchLearningInsights">
            <BenchLearningInsights
              history={history}
              learningEvents={activeRunLearning}
              isRunning={activeRun?.status === 'running'}
            />
          </ComponentErrorBoundary>
        )}

      </div>
      </DataSurface>
    </div>
  );
}
