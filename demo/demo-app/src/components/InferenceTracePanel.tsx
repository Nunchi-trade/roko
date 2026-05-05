import { useState, useRef } from 'react';
import type { InferenceCall, InferenceTraceTotals } from '../hooks/useInferenceTrace';
import { useCanvasSetup } from '../hooks/useCanvasSetup';
import { getCssVar, hexToRgba } from '../lib/color';
import Mosaic, { MosaicCell } from './Mosaic';
import TraceAnnotation from './inference/TraceAnnotation';
import RevealWhen from './RevealWhen';
import Pane from './Pane';
import './InferenceTracePanel.css';

// ── Helpers ──────────────────────────────────────────────────

function fmtCost(n: number): string {
  if (n < 0.001) return `$${n.toFixed(4)}`;
  if (n < 0.01) return `$${n.toFixed(3)}`;
  return `$${n.toFixed(2)}`;
}

function fmtTokens(n: number): string {
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

function fmtLatency(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.round(ms)}ms`;
}

// ── Sparkline ────────────────────────────────────────────────

function CostSparkline({ series }: { series: number[] }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useCanvasSetup(canvasRef, (ctx, w, h) => {
    const lineColor = getCssVar('--rose-bright') || '#e05a7a';
    const pad = 4;
    const plotW = w - pad * 2;
    const plotH = h - pad * 2;

    ctx.clearRect(0, 0, w, h);

    if (series.length < 2) {
      ctx.fillStyle = getCssVar('--text-ghost') || '#666';
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText('waiting for data…', w / 2, h / 2);
      return;
    }

    const maxVal = Math.max(...series, 0.001);
    const getX = (i: number) => pad + (i / Math.max(series.length - 1, 1)) * plotW;
    const getY = (v: number) => pad + plotH - (v / maxVal) * plotH;

    // Gradient fill
    const gradient = ctx.createLinearGradient(0, pad, 0, pad + plotH);
    gradient.addColorStop(0, hexToRgba(lineColor, 0.2));
    gradient.addColorStop(1, hexToRgba(lineColor, 0.02));

    ctx.beginPath();
    for (let i = 0; i < series.length; i++) {
      const x = getX(i);
      const y = getY(series[i]);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.lineTo(getX(series.length - 1), pad + plotH);
    ctx.lineTo(getX(0), pad + plotH);
    ctx.closePath();
    ctx.fillStyle = gradient;
    ctx.fill();

    // Line stroke
    ctx.beginPath();
    for (let i = 0; i < series.length; i++) {
      const x = getX(i);
      const y = getY(series[i]);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.strokeStyle = lineColor;
    ctx.lineWidth = 1.5;
    ctx.lineJoin = 'round';
    ctx.stroke();

    // Dot on last point with glow
    const lastX = getX(series.length - 1);
    const lastY = getY(series[series.length - 1]);
    ctx.beginPath();
    ctx.arc(lastX, lastY, 2.5, 0, Math.PI * 2);
    ctx.fillStyle = lineColor;
    ctx.shadowColor = hexToRgba(lineColor, 0.5);
    ctx.shadowBlur = 6;
    ctx.fill();
    ctx.shadowBlur = 0;
    ctx.shadowColor = 'transparent';
  }, [series]);

  return (
    <div className="trace-panel__sparkline">
      <canvas ref={canvasRef} role="img" aria-label="Cost per inference call" />
    </div>
  );
}

// ── Panel ────────────────────────────────────────────────────

export interface InferenceTracePanelProps {
  calls: InferenceCall[];
  totals: InferenceTraceTotals;
  costSeries: number[];
}

export default function InferenceTracePanel({ calls, totals, costSeries }: InferenceTracePanelProps) {
  const [showCalls, setShowCalls] = useState(false);

  return (
    <RevealWhen visible={calls.length > 0} mode="slide-up">
      <Pane title="TRACE" flat>
        <div className="trace-panel">
          {/* Header with live dot + call count */}
          <div className="trace-panel__header">
            <span className="trace-panel__live-dot" />
            <span>LIVE</span>
            <span className="trace-panel__badge">{totals.calls} call{totals.calls !== 1 ? 's' : ''}</span>
          </div>

          {/* Aggregate metrics */}
          <Mosaic columns={2}>
            <MosaicCell label="COST" value={fmtCost(totals.cost)} mono color="rose" />
            <MosaicCell label="TOKENS" value={fmtTokens(totals.tokens)} mono color="dream" />
            <MosaicCell label="CALLS" value={String(totals.calls)} mono color="bone" />
            <MosaicCell label="LATENCY" value={fmtLatency(totals.avgLatencyMs)} mono color="warning" />
          </Mosaic>

          {/* Cost sparkline */}
          <CostSparkline series={costSeries} />

          {/* Collapsible call list */}
          <button
            className="trace-panel__toggle"
            onClick={() => setShowCalls((prev) => !prev)}
          >
            <span className={`trace-panel__toggle-chevron${showCalls ? ' trace-panel__toggle-chevron--open' : ''}`}>
              ▸
            </span>
            {showCalls ? 'Hide calls' : 'Show calls'}
          </button>

          <div className={`trace-panel__calls${showCalls ? ' trace-panel__calls--open' : ''}`}>
            {[...calls].reverse().map((call, i) => (
              <div key={calls.length - 1 - i} className="trace-panel__call-row">
                <TraceAnnotation
                  tier={call.tier}
                  model={call.model}
                  cost={call.cost}
                  tokens={call.inputTokens + call.outputTokens}
                  latencyMs={call.latencyMs}
                />
              </div>
            ))}
          </div>
        </div>
      </Pane>
    </RevealWhen>
  );
}
