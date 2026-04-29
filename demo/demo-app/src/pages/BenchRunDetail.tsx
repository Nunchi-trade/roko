import { useState, useEffect, useMemo, useRef, useCallback } from 'react';
import { useParams, Link } from 'react-router';
import { useLiveApi } from '../hooks/useLiveApi';
import { getCssVar } from '../lib/color';
import type { BenchRun, BenchTaskResult } from '../lib/bench-types';
import Pane from '../components/Pane';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import TimelineChart from '../components/Charts/TimelineChart';
import HeatmapChart from '../components/Charts/HeatmapChart';
import TaskTable from '../components/TaskTable';
import { ComponentErrorBoundary } from '../components/design';
import './Bench.css';

/* ═══════════════════════════════════════════════════════════
   Shared animation helpers
   ═══════════════════════════════════════════════════════════ */

/** Animate a number counting up from 0 to `target` over `durationMs`. */
function useCountUp(target: number, durationMs = 900, enabled = true): number {
  const [value, setValue] = useState(0);
  const rafRef = useRef(0);

  useEffect(() => {
    if (!enabled || target === 0) {
      setValue(target);
      return;
    }
    const start = performance.now();
    const tick = (now: number) => {
      const elapsed = now - start;
      const progress = Math.min(elapsed / durationMs, 1);
      // ease-out cubic
      const eased = 1 - Math.pow(1 - progress, 3);
      setValue(target * eased);
      if (progress < 1) rafRef.current = requestAnimationFrame(tick);
    };
    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
  }, [target, durationMs, enabled]);

  return value;
}

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
   Animated Gate Badge (SVG path-draw checkmark / X)
   ═══════════════════════════════════════════════════════════ */

function GateBadge({ passed, gate, delay = 0 }: { passed: boolean; gate: string; delay?: number }) {
  const visible = useStagger(delay);

  return (
    <span
      className={`gate-pill ${passed ? 'gate-pass' : 'gate-fail'}`}
      title={`${gate}: ${passed ? 'PASS' : 'FAIL'}`}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        opacity: visible ? 1 : 0,
        transform: visible ? 'scale(1)' : 'scale(0.5)',
        transition: 'opacity 400ms var(--ease), transform 400ms var(--ease)',
      }}
    >
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
        {passed ? (
          <path
            d="M2 5.5 L4 7.5 L8 3"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
            style={{
              strokeDasharray: 12,
              strokeDashoffset: visible ? 0 : 12,
              transition: `stroke-dashoffset 500ms var(--ease) ${delay + 100}ms`,
            }}
          />
        ) : (
          <>
            <path
              d="M2.5 2.5 L7.5 7.5"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              style={{
                strokeDasharray: 8,
                strokeDashoffset: visible ? 0 : 8,
                transition: `stroke-dashoffset 400ms var(--ease) ${delay + 80}ms`,
              }}
            />
            <path
              d="M7.5 2.5 L2.5 7.5"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              style={{
                strokeDasharray: 8,
                strokeDashoffset: visible ? 0 : 8,
                transition: `stroke-dashoffset 400ms var(--ease) ${delay + 160}ms`,
              }}
            />
          </>
        )}
      </svg>
    </span>
  );
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
    'gpt-5.4-mini': [0.00015, 0.0006],
    'gpt-5.4': [0.005, 0.015],
    'gpt-5.3-codex': [0.003, 0.012],
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
  const [animProgress, setAnimProgress] = useState(0);

  // Animate bars from 0 to full on mount / groupBy change
  useEffect(() => {
    setAnimProgress(0);
    const start = performance.now();
    const duration = 700;
    let raf = 0;
    const tick = (now: number) => {
      const t = Math.min((now - start) / duration, 1);
      const eased = 1 - Math.pow(1 - t, 3);
      setAnimProgress(eased);
      if (t < 1) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [segments]);

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
    const bone = getCssVar('--bone');
    const roseDim = getCssVar('--rose-bright');
    const labelColor = getCssVar('--text-soft');
    const mutedColor = getCssVar('--text-dim');
    const costColor = getCssVar('--text-strong');

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
      // Per-bar stagger: each bar starts slightly later
      const barDelay = index * 0.04;
      const barProgress = Math.max(0, Math.min(1, (animProgress - barDelay) / (1 - barDelay)));

      const y = pad.top + index * (barHeight + rowGap);
      const centerY = y + barHeight / 2;
      const total = segment.inputCost + segment.outputCost;
      const inputWidth = (segment.inputCost / maxCost) * plotW * barProgress;
      const outputWidth = (segment.outputCost / maxCost) * plotW * barProgress;
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
      ctx.globalAlpha = barProgress;
      ctx.fillText(fitLabel(ctx, segment.label, pad.left - 12), pad.left - 10, centerY);

      const costText = `$${(total * barProgress).toFixed(3)}`;
      ctx.font = '11px "JetBrains Mono", monospace';
      ctx.textAlign = 'left';
      ctx.fillStyle = costColor;
      const costTextWidth = ctx.measureText(costText).width;
      const costX = Math.max(pad.left + 4, Math.min(barEnd + 8, pad.left + plotW - costTextWidth));
      ctx.fillText(costText, costX, centerY);
      ctx.globalAlpha = 1;
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
  }, [segments, animProgress]);

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
          <canvas ref={canvasRef} className="chart-canvas" role="img" aria-label="Task duration distribution chart" />
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
  const [animProgress, setAnimProgress] = useState(0);

  useEffect(() => {
    setAnimProgress(0);
    const start = performance.now();
    const duration = 800;
    let raf = 0;
    const tick = (now: number) => {
      const t = Math.min((now - start) / duration, 1);
      const eased = 1 - Math.pow(1 - t, 3);
      setAnimProgress(eased);
      if (t < 1) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [results]);

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
    const inColor = getCssVar('--success');
    const outColor = getCssVar('--bone');

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
      const barDelay = index * 0.03;
      const barProgress = Math.max(0, Math.min(1, (animProgress - barDelay) / (1 - barDelay * results.length > 0.5 ? 0.5 : 1)));

      const y = pad.top + index * (barHeight + rowGap);
      const centerY = y + barHeight / 2;
      const inW = (r.tokens_in / maxTokens) * plotW * barProgress;
      const outW = (r.tokens_out / maxTokens) * plotW * barProgress;

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
      ctx.fillStyle = getCssVar('--text-dim');
      ctx.globalAlpha = barProgress;
      const nameText = r.task_name.length > 12 ? r.task_name.slice(0, 11) + '\u2026' : r.task_name;
      ctx.fillText(nameText, pad.left - 8, centerY);

      // Token count
      const total = Math.round((r.tokens_in + r.tokens_out) * barProgress);
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'left';
      ctx.fillStyle = getCssVar('--text-soft');
      const barEnd = pad.left + inW + outW;
      ctx.fillText(total.toLocaleString(), Math.min(barEnd + 6, pad.left + plotW - 40), centerY);
      ctx.globalAlpha = 1;
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
      ctx.fillStyle = getCssVar('--text-dim');
      ctx.fillText(item.label, legendX + 14, legendY);
      legendX += ctx.measureText(item.label).width + 34;
    });
  }, [results, animProgress]);

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
      <canvas ref={canvasRef} className="chart-canvas" role="img" aria-label="Gate pass rate chart" />
    </div>
  );
}

/* ═══════════════════════════════════════════════════════════
   Output Preview Panel (with expand animation + syntax highlight fade)
   ═══════════════════════════════════════════════════════════ */

function OutputPreviewPanel({ results }: { results: BenchTaskResult[] }) {
  const failedWithOutput = results.filter((r) => r.status === 'fail' && (r.output_preview || r.error));
  const passedWithOutput = results.filter((r) => r.status === 'pass' && r.output_preview);

  const [expandedIds, setExpandedIds] = useState<Set<string>>(() => {
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
      {allTasks.map((r, i) => {
        const isExpanded = expandedIds.has(r.task_id);
        return (
          <div
            key={r.task_id}
            style={{
              borderRadius: 6,
              border: '1px solid var(--glass-border)',
              overflow: 'hidden',
              opacity: 1,
              animation: `fadeUp 400ms var(--ease) ${i * 60}ms both`,
            }}
          >
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
                fontSize: 14,
                color: 'var(--text-primary)',
              }}
            >
              <span
                style={{
                  color: 'var(--text-dim)',
                  fontSize: 15,
                  width: 12,
                  display: 'inline-block',
                  transition: 'transform 300ms var(--ease)',
                  transform: isExpanded ? 'rotate(90deg)' : 'rotate(0deg)',
                }}
              >
                {'\u25B6'}
              </span>
              <span className={`status-badge status-${r.status}`} style={{ fontSize: 15 }}>
                {r.status.toUpperCase()}
              </span>
              <span style={{ flex: 1, color: 'var(--text-strong)' }}>{r.task_name}</span>
              {/* Gate verdict badges */}
              {r.gate_verdicts.length > 0 && (
                <span style={{ display: 'flex', gap: 2 }}>
                  {r.gate_verdicts.map((g, gi) => (
                    <GateBadge
                      key={g.gate}
                      gate={g.gate}
                      passed={g.passed}
                      delay={isExpanded ? gi * 80 : 0}
                    />
                  ))}
                </span>
              )}
            </button>
            <div
              style={{
                maxHeight: isExpanded ? 400 : 0,
                opacity: isExpanded ? 1 : 0,
                overflow: 'hidden',
                transition: 'max-height 400ms var(--ease), opacity 300ms var(--ease)',
              }}
            >
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
                    fontSize: 13,
                    color: 'var(--text-primary)',
                    overflow: 'auto',
                    maxHeight: 200,
                    whiteSpace: 'pre-wrap',
                    wordBreak: 'break-word',
                    animation: isExpanded ? 'fadeIn 500ms var(--ease) 200ms both' : 'none',
                  }}>
                    {r.output_preview}
                  </pre>
                )}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

/* ═══════════════════════════════════════════════════════════
   Section wrapper with crossfade mount animation
   ═══════════════════════════════════════════════════════════ */

function AnimatedSection({ delay, children }: { delay: number; children: React.ReactNode }) {
  const visible = useStagger(delay);
  return (
    <div
      style={{
        opacity: visible ? 1 : 0,
        transform: visible ? 'translateY(0)' : 'translateY(16px)',
        transition: 'opacity 500ms var(--ease), transform 500ms var(--ease)',
      }}
    >
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
    <span
      style={{
        display: 'inline-block',
        padding: '2px 8px',
        borderRadius: 'var(--radius-sm)',
        background: 'var(--glass-bg)',
        border: '1px solid var(--glass-border)',
        fontFamily: 'var(--mono)',
        fontSize: 'var(--text-sm)',
        color: 'var(--rose-bright)',
        opacity: visible ? 1 : 0,
        transform: visible ? 'translateX(0)' : 'translateX(-12px)',
        transition: 'opacity 400ms var(--ease), transform 400ms var(--ease)',
      }}
    >
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

  useEffect(() => {
    if (!id) return;
    (async () => {
      try {
        const data = await get<BenchRun>(`/api/bench/runs/${id}`);
        if (data && data.id) {
          setRun(data);
          return;
        }
      } catch { /* show not-found */ }
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
  const tokenEffRaw = totalTokens > 0 ? passCount / (totalTokens / 1000) : 0;

  return (
    <div className="bench-page">
      {/* ── Hero ── */}
      <div className="bench-hero">
        <div className="bench-hero-header">
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <Link
              to="/bench"
              className="bench-back"
              style={{
                animation: 'scaleIn 400ms var(--ease) both',
                display: 'inline-block',
                transition: 'color 200ms var(--ease), transform 200ms var(--ease)',
              }}
              onMouseEnter={(e) => {
                e.currentTarget.style.transform = 'translateY(-1px) scale(1.05)';
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.transform = 'translateY(0) scale(1)';
              }}
            >
              &larr; Back
            </Link>
            <h1 className="bench-page-title">Run {run.id.slice(0, 8)}</h1>
          </div>
          <p className="bench-page-sub" style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
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
              <OutputPreviewPanel results={run.results} />
            </Pane>
          </ComponentErrorBoundary>
        </AnimatedSection>

        {/* ── Compare Button ── */}
        <AnimatedSection delay={700}>
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
