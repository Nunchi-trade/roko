import { useState, useEffect, useMemo, useRef, useCallback } from 'react';
import { useParams, Link } from 'react-router';
import { useApiWithFallback } from '../hooks/useApiWithFallback';
import type { BenchRun, BenchTaskResult } from '../lib/bench-types';
import Pane from '../components/Pane';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import TimelineChart from '../components/Charts/TimelineChart';
import HeatmapChart from '../components/Charts/HeatmapChart';
import TaskTable from '../components/TaskTable';
import './Bench.css';

/* ═══════════════════════════════════════════════════════════
   Cost Breakdown Chart (inline — complex canvas component)
   ═══════════════════════════════════════════════════════════ */

type CostGroupBy = 'task' | 'model' | 'difficulty';

type CostSegment = {
  label: string;
  inputCost: number;
  outputCost: number;
};

function tokenCost(model: string, tokensIn: number, tokensOut: number): [number, number] {
  const rates: Record<string, [number, number]> = {
    haiku: [0.00025, 0.00125],
    sonnet: [0.003, 0.015],
    opus: [0.015, 0.075],
    'gpt-4o-mini': [0.00015, 0.0006],
    'gpt-4o': [0.005, 0.015],
    'o3-mini': [0.0011, 0.0044],
    gemini: [0.00125, 0.01],
  };
  const key = Object.keys(rates).find((k) => model.includes(k)) ?? 'sonnet';
  const [inRate, outRate] = rates[key];
  return [(Math.max(tokensIn, 0) * inRate) / 1000, (Math.max(tokensOut, 0) * outRate) / 1000];
}

function fitLabel(ctx: CanvasRenderingContext2D, text: string, maxWidth: number) {
  if (maxWidth <= 0) return '';
  if (ctx.measureText(text).width <= maxWidth) return text;

  let value = text;
  while (value.length > 1 && ctx.measureText(`${value}...`).width > maxWidth) {
    value = value.slice(0, -1);
  }
  return `${value}...`;
}

function truncateLabel(text: string, maxLength: number) {
  if (text.length <= maxLength) return text;
  if (maxLength <= 3) return text.slice(0, maxLength);
  return `${text.slice(0, maxLength - 3)}...`;
}

function buildCostSegments(results: BenchTaskResult[], groupBy: CostGroupBy): CostSegment[] {
  if (results.length === 0) return [];

  const addCost = (segment: CostSegment, result: BenchTaskResult) => {
    const [inputCost, outputCost] = tokenCost(result.model, result.tokens_in, result.tokens_out);
    segment.inputCost += inputCost;
    segment.outputCost += outputCost;
  };

  if (groupBy === 'task') {
    return results.map((result) => {
      const [inputCost, outputCost] = tokenCost(result.model, result.tokens_in, result.tokens_out);
      return {
        label: truncateLabel(result.task_name, 25),
        inputCost,
        outputCost,
      };
    });
  }

  if (groupBy === 'model') {
    const grouped = new Map<string, CostSegment>();
    for (const result of results) {
      const label = result.model || 'unknown';
      const segment = grouped.get(label) ?? { label, inputCost: 0, outputCost: 0 };
      addCost(segment, result);
      grouped.set(label, segment);
    }
    return Array.from(grouped.values());
  }

  const ordered = [...results].sort(
    (a, b) => a.duration_ms - b.duration_ms || a.task_name.localeCompare(b.task_name),
  );
  const buckets = new Map<'Easy' | 'Medium' | 'Hard', CostSegment>([
    ['Easy', { label: 'Easy', inputCost: 0, outputCost: 0 }],
    ['Medium', { label: 'Medium', inputCost: 0, outputCost: 0 }],
    ['Hard', { label: 'Hard', inputCost: 0, outputCost: 0 }],
  ]);
  const easyCut = Math.ceil(ordered.length / 3);
  const mediumCut = Math.ceil((ordered.length * 2) / 3);

  ordered.forEach((result, index) => {
    const label = index < easyCut ? 'Easy' : index < mediumCut ? 'Medium' : 'Hard';
    const segment = buckets.get(label);
    if (segment) addCost(segment, result);
  });

  return (['Easy', 'Medium', 'Hard'] as const)
    .map((label) => buckets.get(label))
    .filter((segment): segment is CostSegment => Boolean(segment));
}

function CostBreakdownChart({ results, height = 280 }: { results: BenchTaskResult[]; height?: number }) {
  const [groupBy, setGroupBy] = useState<CostGroupBy>('task');
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const segments = useMemo(() => buildCostSegments(results, groupBy), [results, groupBy]);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || segments.length === 0) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;

    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.scale(dpr, dpr);

    const w = rect.width;
    const h = rect.height;
    const pad = { top: 12, right: 16, bottom: 24, left: 120 };
    const legendSpace = 20;
    const plotW = w - pad.left - pad.right;
    const plotH = h - pad.top - pad.bottom - legendSpace;
    if (plotW <= 0 || plotH <= 0) return;

    ctx.clearRect(0, 0, w, h);

    const maxCost = Math.max(...segments.map((segment) => segment.inputCost + segment.outputCost), 0.001);
    const rowGap = 4;
    const barHeight = Math.max(4, Math.min(24, plotH / segments.length - rowGap));
    const bone = '#C8B890';
    const roseDim = '#D4918F';
    const labelColor = '#BDAEB8';
    const mutedColor = '#85707D';
    const costColor = '#F0E4E3';

    ctx.lineWidth = 1;
    ctx.strokeStyle = 'rgba(255,255,255,0.05)';
    for (let i = 0; i <= 4; i += 1) {
      const x = pad.left + (plotW * i) / 4;
      ctx.beginPath();
      ctx.moveTo(x, pad.top - 2);
      ctx.lineTo(x, pad.top + plotH);
      ctx.stroke();
    }

    segments.forEach((segment, index) => {
      const y = pad.top + index * (barHeight + rowGap);
      const centerY = y + barHeight / 2;
      const total = segment.inputCost + segment.outputCost;
      const inputWidth = (segment.inputCost / maxCost) * plotW;
      const outputWidth = (segment.outputCost / maxCost) * plotW;
      const barEnd = pad.left + inputWidth + outputWidth;

      ctx.fillStyle = 'rgba(255,255,255,0.04)';
      ctx.fillRect(pad.left, y, plotW, barHeight);

      if (inputWidth > 0) {
        ctx.fillStyle = bone;
        ctx.fillRect(pad.left, y, inputWidth, barHeight);
      }

      if (outputWidth > 0) {
        ctx.fillStyle = roseDim;
        ctx.fillRect(pad.left + inputWidth, y, outputWidth, barHeight);
      }

      ctx.font = '11px "General Sans", sans-serif';
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.fillStyle = labelColor;
      ctx.fillText(fitLabel(ctx, segment.label, pad.left - 12), pad.left - 10, centerY);

      const costText = `$${total.toFixed(3)}`;
      ctx.font = '11px "JetBrains Mono", monospace';
      ctx.textAlign = 'left';
      ctx.fillStyle = costColor;
      const costTextWidth = ctx.measureText(costText).width;
      const costX = Math.max(pad.left + 4, Math.min(barEnd + 8, pad.left + plotW - costTextWidth));
      ctx.fillText(costText, costX, centerY);
    });

    const legendY = h - 9;
    ctx.font = '10px "General Sans", sans-serif';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    const legendItems = [
      { color: bone, label: 'Input tokens' },
      { color: roseDim, label: 'Output tokens' },
    ];
    let legendX = pad.left;
    legendItems.forEach((item) => {
      ctx.fillStyle = item.color;
      ctx.fillRect(legendX, legendY - 5, 10, 10);
      ctx.fillStyle = mutedColor;
      ctx.fillText(item.label, legendX + 14, legendY);
      legendX += ctx.measureText(item.label).width + 34;
    });
  }, [segments]);

  useEffect(() => {
    draw();
    const canvas = canvasRef.current;
    if (!canvas || typeof ResizeObserver === 'undefined') return undefined;

    const ro = new ResizeObserver(() => {
      draw();
    });
    ro.observe(canvas);
    return () => ro.disconnect();
  }, [draw]);

  return (
    <div className="chart-container" style={{ height, display: 'flex', flexDirection: 'column' }}>
      <div style={{ display: 'flex', gap: 8, marginBottom: 8, flex: '0 0 auto' }}>
        {(['task', 'model', 'difficulty'] as const).map((mode) => (
          <label key={mode} className="gate-toggle">
            <input
              type="radio"
              name="costGroup"
              checked={groupBy === mode}
              onChange={() => setGroupBy(mode)}
            />
            <span>{mode.charAt(0).toUpperCase() + mode.slice(1)}</span>
          </label>
        ))}
      </div>
      <div style={{ flex: 1, minHeight: 0 }}>
        {segments.length === 0 ? (
          <div style={{ height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
            <p className="bench-empty-text">No cost data available.</p>
          </div>
        ) : (
          <canvas ref={canvasRef} className="chart-canvas" />
        )}
      </div>
    </div>
  );
}

/* ═══════════════════════════════════════════════════════════
   Token Flow Chart (inline horizontal stacked bars)
   ═══════════════════════════════════════════════════════════ */

function TokenFlowChart({ results, height = 280 }: { results: BenchTaskResult[]; height?: number }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || results.length === 0) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;

    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.scale(dpr, dpr);

    const w = rect.width;
    const h = rect.height;
    const pad = { top: 8, right: 16, bottom: 24, left: 100 };
    const plotW = w - pad.left - pad.right;
    const plotH = h - pad.top - pad.bottom;
    if (plotW <= 0 || plotH <= 0) return;

    ctx.clearRect(0, 0, w, h);

    const maxTokens = Math.max(...results.map((r) => r.tokens_in + r.tokens_out), 1);
    const rowGap = 4;
    const barHeight = Math.max(4, Math.min(22, plotH / results.length - rowGap));
    const inColor = '#7A8A78'; // --success muted
    const outColor = '#C8B890'; // --bone

    // Grid lines
    ctx.lineWidth = 1;
    ctx.strokeStyle = 'rgba(255,255,255,0.05)';
    for (let i = 0; i <= 4; i++) {
      const x = pad.left + (plotW * i) / 4;
      ctx.beginPath();
      ctx.moveTo(x, pad.top);
      ctx.lineTo(x, pad.top + plotH);
      ctx.stroke();
    }

    results.forEach((r, index) => {
      const y = pad.top + index * (barHeight + rowGap);
      const centerY = y + barHeight / 2;
      const inW = (r.tokens_in / maxTokens) * plotW;
      const outW = (r.tokens_out / maxTokens) * plotW;

      // Background
      ctx.fillStyle = 'rgba(255,255,255,0.03)';
      ctx.fillRect(pad.left, y, plotW, barHeight);

      // Input bar
      if (inW > 0) {
        ctx.fillStyle = inColor;
        ctx.fillRect(pad.left, y, inW, barHeight);
      }

      // Output bar
      if (outW > 0) {
        ctx.fillStyle = outColor;
        ctx.fillRect(pad.left + inW, y, outW, barHeight);
      }

      // Label
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.fillStyle = '#8a7a88';
      const nameText = r.task_name.length > 12 ? r.task_name.slice(0, 11) + '\u2026' : r.task_name;
      ctx.fillText(nameText, pad.left - 8, centerY);

      // Token count
      const total = r.tokens_in + r.tokens_out;
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'left';
      ctx.fillStyle = '#BDAEB8';
      const barEnd = pad.left + inW + outW;
      ctx.fillText(total.toLocaleString(), Math.min(barEnd + 6, pad.left + plotW - 40), centerY);
    });

    // Legend
    const legendY = h - 8;
    ctx.font = '10px "General Sans", sans-serif';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    const legendItems = [
      { color: inColor, label: 'Tokens in' },
      { color: outColor, label: 'Tokens out' },
    ];
    let legendX = pad.left;
    legendItems.forEach((item) => {
      ctx.fillStyle = item.color;
      ctx.fillRect(legendX, legendY - 5, 10, 10);
      ctx.fillStyle = '#85707D';
      ctx.fillText(item.label, legendX + 14, legendY);
      legendX += ctx.measureText(item.label).width + 34;
    });
  }, [results]);

  useEffect(() => {
    draw();
    const canvas = canvasRef.current;
    if (!canvas || typeof ResizeObserver === 'undefined') return undefined;

    const ro = new ResizeObserver(draw);
    ro.observe(canvas);
    return () => ro.disconnect();
  }, [draw]);

  if (results.length === 0) {
    return (
      <div className="chart-container" style={{ height, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        <p className="bench-empty-text">No token data.</p>
      </div>
    );
  }

  return (
    <div className="chart-container" style={{ height }}>
      <canvas ref={canvasRef} className="chart-canvas" />
    </div>
  );
}

/* ═══════════════════════════════════════════════════════════
   Output Preview Panel
   ═══════════════════════════════════════════════════════════ */

function OutputPreviewPanel({ results }: { results: BenchTaskResult[] }) {
  const failedWithOutput = results.filter((r) => r.status === 'fail' && (r.output_preview || r.error));
  const passedWithOutput = results.filter((r) => r.status === 'pass' && r.output_preview);

  const [expandedIds, setExpandedIds] = useState<Set<string>>(() => {
    // Auto-expand failed tasks
    return new Set(failedWithOutput.map((r) => r.task_id));
  });

  const toggle = (id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  if (failedWithOutput.length === 0 && passedWithOutput.length === 0) {
    return <p className="bench-empty-text">No output previews available.</p>;
  }

  const allTasks = [...failedWithOutput, ...passedWithOutput];

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      {allTasks.map((r) => (
        <div key={r.task_id} style={{ borderRadius: 6, border: '1px solid var(--glass-border)', overflow: 'hidden' }}>
          <button
            onClick={() => toggle(r.task_id)}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              width: '100%',
              padding: '8px 12px',
              background: 'rgba(255,255,255,0.02)',
              border: 'none',
              cursor: 'pointer',
              textAlign: 'left',
              fontFamily: 'var(--mono)',
              fontSize: 11,
              color: 'var(--text-primary)',
            }}
          >
            <span style={{ color: 'var(--text-dim)', fontSize: 9, width: 12 }}>
              {expandedIds.has(r.task_id) ? '\u25BC' : '\u25B6'}
            </span>
            <span className={`status-badge status-${r.status}`} style={{ fontSize: 8 }}>
              {r.status.toUpperCase()}
            </span>
            <span style={{ flex: 1, color: 'var(--text-strong)' }}>{r.task_name}</span>
          </button>
          {expandedIds.has(r.task_id) && (
            <div style={{ padding: '0 12px 12px' }}>
              {r.error && (
                <div className="task-error" style={{ marginTop: 8 }}>{r.error}</div>
              )}
              {r.output_preview && (
                <pre className="task-output-code" style={{
                  marginTop: 8,
                  padding: 10,
                  background: 'rgba(0,0,0,0.3)',
                  borderRadius: 4,
                  fontSize: 10,
                  color: 'var(--text-primary)',
                  overflow: 'auto',
                  maxHeight: 200,
                  whiteSpace: 'pre-wrap',
                  wordBreak: 'break-word',
                }}>
                  {r.output_preview}
                </pre>
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

/* ═══════════════════════════════════════════════════════════
   BenchRunDetail — Main Page
   ═══════════════════════════════════════════════════════════ */

export default function BenchRunDetail() {
  const { id } = useParams<{ id: string }>();
  const { get } = useApiWithFallback();
  const [run, setRun] = useState<BenchRun | null>(null);

  useEffect(() => {
    if (!id) return;
    (async () => {
      try {
        const data = await get<BenchRun>(`/api/bench/runs/${id}`);
        if (data && data.id) {
          setRun(data);
          return;
        }
      } catch { /* no fallback -- show not-found */ }
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
        <div className="bench-body">
          <div className="bench-empty">
            <p className="bench-empty-text">Loading run {id}...</p>
          </div>
        </div>
      </div>
    );
  }

  const summary = run.summary;

  // Computed hero metrics
  const totalTokens = run.results.reduce((s, r) => s + r.tokens_in + r.tokens_out, 0);
  const passCount = run.results.filter((r) => r.status === 'pass').length;
  const tokenEfficiency = totalTokens > 0 ? (passCount / (totalTokens / 1000)).toFixed(2) : '0';

  return (
    <div className="bench-page">
      {/* ── Hero ── */}
      <div className="bench-hero">
        <div className="bench-hero-header">
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <Link to="/bench" className="bench-back">&larr; Back</Link>
            <h1 className="bench-page-title">Run {run.id.slice(0, 8)}</h1>
          </div>
          <p className="bench-page-sub">
            {run.suite_name} &middot; {run.config.model} &middot; {run.config.strategy.replace(/_/g, ' ')}
          </p>
        </div>
        {summary && (
          <div className="bench-hero-stats">
            <Mosaic columns={6}>
              <MosaicCell label="PASS RATE" value={`${(summary.pass_rate * 100).toFixed(0)}%`} color="success" />
              <MosaicCell label="TOTAL COST" value={`$${summary.total_cost_usd.toFixed(3)}`} color="warning" />
              <MosaicCell label="USD/SUCCESS" value={`$${summary.cost_per_success_usd.toFixed(3)}`} color="bone" mono />
              <MosaicCell label="AVG DURATION" value={`${(summary.avg_duration_ms / 1000).toFixed(1)}s`} color="dream" mono />
              <MosaicCell label="TOTAL TOKENS" value={totalTokens.toLocaleString()} color="rose" mono />
              <MosaicCell
                label="TOKEN EFF"
                value={tokenEfficiency}
                sub="passes/1K tok"
                color="success"
                mono
              />
            </Mosaic>
          </div>
        )}
      </div>

      <div className="bench-body">
        {/* ── Task Timeline Waterfall ── */}
        {timelineTasks.length > 0 && (
          <Pane title="TASK TIMELINE">
            <TimelineChart
              tasks={timelineTasks}
              height={Math.max(200, timelineTasks.length * 28 + 48)}
            />
          </Pane>
        )}

        {/* ── Task Results Table ── */}
        <Pane title="TASK RESULTS">
          <TaskTable results={run.results} showDifficulty showOutputPreview />
        </Pane>

        {/* ── Cost Attribution ── */}
        <Pane title="COST ATTRIBUTION">
          <CostBreakdownChart
            results={run.results}
            height={Math.max(280, run.results.length * 28 + 48)}
          />
        </Pane>

        {/* ── Token Flow ── */}
        <Pane title="TOKEN FLOW">
          <TokenFlowChart
            results={run.results}
            height={Math.max(200, run.results.length * 26 + 40)}
          />
        </Pane>

        {/* ── Gate Heatmap ── */}
        {gateNames.length > 0 && (
          <Pane title="GATE HEATMAP">
            <HeatmapChart
              rows={heatRows}
              columns={gateNames}
              values={heatValues}
              height={Math.max(200, heatRows.length * 28 + 48)}
            />
          </Pane>
        )}

        {/* ── Output Previews ── */}
        <Pane title="OUTPUT PREVIEWS">
          <OutputPreviewPanel results={run.results} />
        </Pane>

        {/* ── Compare Button ── */}
        <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: 16, marginBottom: 8 }}>
          <Link
            to={`/bench/compare?ids=${run.id}`}
            className="btn btn-sm"
            style={{
              textDecoration: 'none',
              display: 'inline-flex',
              alignItems: 'center',
              gap: 6,
            }}
          >
            Compare with...
          </Link>
        </div>

        {/* ── Configuration ── */}
        <Pane title="CONFIGURATION">
          <div className="config-detail-grid">
            <div><span className="detail-label">Model:</span> {run.config.model}</div>
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
      </div>
    </div>
  );
}
