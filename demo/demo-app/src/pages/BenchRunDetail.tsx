import { useState, useEffect, useMemo } from 'react';
import { useParams, Link } from 'react-router';
import { useLiveApi } from '../hooks/useLiveApi';
import { useCountUp } from '../hooks/useCountUp';
import type { BenchRun } from '../lib/bench-types';
import Pane from '../components/Pane';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import TimelineChart from '../components/Charts/TimelineChart';
import HeatmapChart from '../components/Charts/HeatmapChart';
import TaskTable from '../components/TaskTable';
import { ComponentErrorBoundary, DataSurface } from '../components/design';
import { CostBreakdownChart } from '../components/evaluate/CostBreakdownChart';
import { TokenFlowChart } from '../components/evaluate/TokenFlowChart';
import { OutputPreview } from '../components/evaluate/OutputPreview';
import './Bench.css';
import '../styles/bench-compare.css';
import '../styles/bench-race.css';
import './BenchRunDetail.css';

/* ═══════════════════════════════════════════════════════════
   Shared animation helpers
   ═══════════════════════════════════════════════════════════ */

/** Returns true after `delayMs` to trigger staggered mounts. */
function useStagger(delayMs: number): boolean {
  const [ready, setReady] = useState(false);
  useEffect(() => {
    const t = setTimeout(() => setReady(true), delayMs);
    return () => clearTimeout(t);
  }, [delayMs]);
  return ready;
}

/* ═══════════════════════════════════════════════════════════
   Animated Stat Cell (count-up values)
   ═══════════════════════════════════════════════════════════ */

type MosaicColor = 'rose' | 'bone' | 'dream' | 'success' | 'warning';

function AnimatedStatCell({
  label,
  rawValue,
  format,
  color,
  sub,
  mono,
  delay = 0,
}: {
  label: string;
  rawValue: number;
  format: (v: number) => string;
  color: MosaicColor;
  sub?: string;
  mono?: boolean;
  delay?: number;
}) {
  const mounted = useStagger(delay);
  const animated = useCountUp(rawValue, 800, mounted);

  return <MosaicCell label={label} value={format(animated)} color={color} sub={sub} mono={mono} />;
}

/* ═══════════════════════════════════════════════════════════
   Section wrapper with crossfade mount animation
   ═══════════════════════════════════════════════════════════ */

function AnimatedSection({ delay, children }: { delay: number; children: React.ReactNode }) {
  const visible = useStagger(delay);
  return (
    <div className={`bench-animated-section ${visible ? 'bench-animated-section--visible' : 'bench-animated-section--hidden'}`}>
      {children}
    </div>
  );
}

/* ═══════════════════════════════════════════════════════════
   Model Badge (slide-in)
   ═══════════════════════════════════════════════════════════ */

function ModelBadge({ model, delay = 0 }: { model: string; delay?: number }) {
  const visible = useStagger(delay);
  return (
    <span className={`bench-model-badge ${visible ? 'bench-model-badge--visible' : 'bench-model-badge--hidden'}`}>
      {model}
    </span>
  );
}

/* ═══════════════════════════════════════════════════════════
   BenchRunDetail — Main Page
   ═══════════════════════════════════════════════════════════ */

export default function BenchRunDetail() {
  const { id } = useParams<{ id: string }>();
  const { get } = useLiveApi();
  const [run, setRun] = useState<BenchRun | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!id) return;
    setLoading(true);
    setError(null);
    (async () => {
      try {
        const data = await get<BenchRun>(`/api/bench/runs/${id}`);
        if (data && data.id) {
          setRun(data);
          return;
        }
        setError(`Run ${id} not found`);
      } catch {
        setError(`Failed to load run ${id}`);
      } finally {
        setLoading(false);
      }
    })();
  }, [id, get]);

  // Timeline tasks (waterfall)
  const timelineTasks = useMemo(() => {
    if (!run) return [];
    return run.results.map((r, i) => ({
      name: r.task_name,
      startMs: run.results.slice(0, i).reduce((s, prev) => s + prev.duration_ms, 0),
      durationMs: r.duration_ms,
      status: r.status as 'pass' | 'fail' | 'pending' | 'running' | 'skipped',
      gateVerdicts: r.gate_verdicts.map((g) => ({ gate: g.gate, passed: g.passed })),
    }));
  }, [run]);

  // Gate heatmap data
  const { gateNames, heatRows, heatValues } = useMemo(() => {
    if (!run) return { gateNames: [], heatRows: [], heatValues: [] };
    const names = [...new Set(run.results.flatMap((r) => r.gate_verdicts.map((g) => g.gate)))];
    const rows = run.results.map((r) => r.task_name);
    const values = run.results.map((r) =>
      names.map((gate) => {
        const v = r.gate_verdicts.find((g) => g.gate === gate);
        return v ? v.passed : null;
      }),
    );
    return { gateNames: names, heatRows: rows, heatValues: values };
  }, [run]);

  if (!run) {
    return (
      <div className="bench-page">
        <DataSurface loading={loading} error={error} empty={!loading && !error} emptyLabel={`Run ${id} not found`}>
          <div />
        </DataSurface>
      </div>
    );
  }

  const summary = run.summary;

  // Computed hero metrics
  const totalTokens = run.results.reduce((s, r) => s + r.tokens_in + r.tokens_out, 0);
  const passCount = run.results.filter((r) => r.status === 'pass').length;
  const tokenEffRaw = totalTokens > 0 ? passCount / (totalTokens / 1000) : 0;

  return (
    <div className="bench-page">
      {/* ── Hero ── */}
      <div className="bench-hero">
        <div className="bench-hero-header">
          <div className="bench-hero-row">
            <Link
              to="/bench"
              className="bench-back bench-back-link"
            >
              &larr; Back
            </Link>
            <h1 className="bench-page-title">Run {run.id.slice(0, 8)}</h1>
          </div>
          <p className="bench-page-sub bench-page-sub-row">
            {run.suite_name} &middot; <ModelBadge model={run.config.model} delay={200} /> &middot; {run.config.strategy.replace(/_/g, ' ')}
          </p>
        </div>
        {summary && (
          <div className="bench-hero-stats">
            <Mosaic columns={6}>
              <AnimatedStatCell
                label="PASS RATE"
                rawValue={summary.pass_rate * 100}
                format={(v) => `${v.toFixed(0)}%`}
                color="success"
                delay={0}
              />
              <AnimatedStatCell
                label="TOTAL COST"
                rawValue={summary.total_cost_usd}
                format={(v) => `$${v.toFixed(3)}`}
                color="warning"
                delay={60}
              />
              <AnimatedStatCell
                label="USD/SUCCESS"
                rawValue={summary.cost_per_success_usd}
                format={(v) => `$${v.toFixed(3)}`}
                color="bone"
                mono
                delay={120}
              />
              <AnimatedStatCell
                label="AVG DURATION"
                rawValue={summary.avg_duration_ms / 1000}
                format={(v) => `${v.toFixed(1)}s`}
                color="dream"
                mono
                delay={180}
              />
              <AnimatedStatCell
                label="TOTAL TOKENS"
                rawValue={totalTokens}
                format={(v) => Math.round(v).toLocaleString()}
                color="rose"
                mono
                delay={240}
              />
              <AnimatedStatCell
                label="TOKEN EFF"
                rawValue={tokenEffRaw}
                format={(v) => v.toFixed(2)}
                color="success"
                mono
                sub="passes/1K tok"
                delay={300}
              />
            </Mosaic>
          </div>
        )}
      </div>

      <div className="bench-body">
        {/* ── Task Timeline Waterfall ── */}
        {timelineTasks.length > 0 && (
          <AnimatedSection delay={100}>
            <ComponentErrorBoundary name="TaskTimeline">
              <Pane title="TASK TIMELINE">
                <TimelineChart
                  tasks={timelineTasks}
                  height={Math.max(200, timelineTasks.length * 28 + 48)}
                />
              </Pane>
            </ComponentErrorBoundary>
          </AnimatedSection>
        )}

        {/* ── Task Results Table ── */}
        <AnimatedSection delay={200}>
          <Pane title="TASK RESULTS">
            <TaskTable results={run.results} showDifficulty showOutputPreview />
          </Pane>
        </AnimatedSection>

        {/* ── Cost Attribution ── */}
        <AnimatedSection delay={300}>
          <ComponentErrorBoundary name="CostBreakdown">
            <Pane title="COST ATTRIBUTION">
              <CostBreakdownChart
                results={run.results}
                height={Math.max(280, run.results.length * 28 + 48)}
              />
            </Pane>
          </ComponentErrorBoundary>
        </AnimatedSection>

        {/* ── Token Flow ── */}
        <AnimatedSection delay={400}>
          <ComponentErrorBoundary name="TokenFlow">
            <Pane title="TOKEN FLOW">
              <TokenFlowChart
                results={run.results}
                height={Math.max(200, run.results.length * 26 + 40)}
              />
            </Pane>
          </ComponentErrorBoundary>
        </AnimatedSection>

        {/* ── Gate Heatmap ── */}
        {gateNames.length > 0 && (
          <AnimatedSection delay={500}>
            <ComponentErrorBoundary name="GateHeatmap">
              <Pane title="GATE HEATMAP">
                <HeatmapChart
                  rows={heatRows}
                  columns={gateNames}
                  values={heatValues}
                  height={Math.max(200, heatRows.length * 28 + 48)}
                />
              </Pane>
            </ComponentErrorBoundary>
          </AnimatedSection>
        )}

        {/* ── Output Previews ── */}
        <AnimatedSection delay={600}>
          <ComponentErrorBoundary name="OutputPreviews">
            <Pane title="OUTPUT PREVIEWS">
              <OutputPreview results={run.results} />
            </Pane>
          </ComponentErrorBoundary>
        </AnimatedSection>

        {/* ── Compare Button ── */}
        <AnimatedSection delay={700}>
          <div className="bench-compare-row">
            <Link
              to={`/bench/compare?ids=${run.id}`}
              className="btn btn-sm bench-compare-link"
            >
              Compare with...
            </Link>
          </div>
        </AnimatedSection>

        {/* ── Configuration ── */}
        <AnimatedSection delay={800}>
          <Pane title="CONFIGURATION">
            <div className="config-detail-grid">
              <div><span className="detail-label">Model:</span> <ModelBadge model={run.config.model} delay={850} /></div>
              <div><span className="detail-label">Provider:</span> {run.config.provider ?? '-'}</div>
              <div><span className="detail-label">Strategy:</span> {run.config.strategy}</div>
              <div><span className="detail-label">Temperature:</span> {run.config.temperature ?? '-'}</div>
              <div><span className="detail-label">Max Tokens:</span> {run.config.max_tokens ?? '-'}</div>
              <div><span className="detail-label">Timeout:</span> {run.config.timeout_secs}s</div>
              <div><span className="detail-label">Retries:</span> {run.config.retries}</div>
              <div><span className="detail-label">Started:</span> {new Date(run.started_at).toLocaleString()}</div>
              {run.finished_at && (
                <div><span className="detail-label">Finished:</span> {new Date(run.finished_at).toLocaleString()}</div>
              )}
            </div>
          </Pane>
        </AnimatedSection>
      </div>
    </div>
  );
}
