import { useCallback, useEffect, useRef, useState } from 'react';
import { useLiveApi } from '../hooks/useLiveApi';
import { useBenchSSE } from '../hooks/useBenchSSE';

export interface CostRaceModel {
  model: string;
  cost_usd: number;
  tokens: number;
  tasks: number;
  color?: string;
}

export interface CostRaceProps {
  /** Live data provided by a parent view. If omitted, the component fetches from the API. */
  models?: CostRaceModel[];
  /** Subscribe to live bench SSE updates. */
  live?: boolean;
  height?: number;
}

interface CostRaceResponse {
  models?: CostRaceModel[];
}

const MODEL_COLORS: Record<string, string> = {
  'claude-sonnet-4': '#C8B890',
  'claude-haiku-3': '#8A9C86',
  'claude-opus-4': '#AA7088',
  'gpt-5.4': '#D8A878',
  'gpt-5.4-mini': '#D8C098',
  'gemini-2.5-pro': '#9A8AB8',
};

function isCostRaceModel(value: unknown): value is CostRaceModel {
  if (!value || typeof value !== 'object') return false;
  const row = value as CostRaceModel;
  return (
    typeof row.model === 'string' &&
    typeof row.cost_usd === 'number' &&
    typeof row.tokens === 'number' &&
    typeof row.tasks === 'number'
  );
}

function normalizeRows(source: unknown): CostRaceModel[] {
  let rows: unknown[] = [];
  if (Array.isArray(source)) {
    rows = source;
  } else if (source && typeof source === 'object') {
    const maybeRows = (source as CostRaceResponse).models;
    if (Array.isArray(maybeRows)) rows = maybeRows;
  }

  return rows
    .filter(isCostRaceModel)
    .map((row) => ({
      model: row.model,
      cost_usd: Number.isFinite(row.cost_usd) ? row.cost_usd : 0,
      tokens: Number.isFinite(row.tokens) ? row.tokens : 0,
      tasks: Number.isFinite(row.tasks) ? row.tasks : 0,
      color: row.color,
    }));
}

function sortRows(rows: CostRaceModel[]): CostRaceModel[] {
  return [...rows].sort((a, b) => b.cost_usd - a.cost_usd || a.model.localeCompare(b.model));
}

function mergeRows(base: CostRaceModel[], incoming: CostRaceModel[]): CostRaceModel[] {
  const map = new Map<string, CostRaceModel>();

  for (const row of base) {
    map.set(row.model, { ...row });
  }

  for (const row of incoming) {
    const existing = map.get(row.model);
    if (!existing) {
      map.set(row.model, { ...row });
      continue;
    }

    map.set(row.model, {
      model: row.model,
      cost_usd: Math.max(existing.cost_usd, row.cost_usd),
      tokens: Math.max(existing.tokens, row.tokens),
      tasks: Math.max(existing.tasks, row.tasks),
      color: row.color ?? existing.color,
    });
  }

  return sortRows([...map.values()]);
}

function hashString(input: string): number {
  let hash = 0;
  for (let i = 0; i < input.length; i += 1) {
    hash = (hash * 31 + input.charCodeAt(i)) | 0;
  }
  return Math.abs(hash);
}

function getModelColor(model: string, explicit?: string): string {
  if (explicit) return explicit;
  for (const [needle, color] of Object.entries(MODEL_COLORS)) {
    if (model.includes(needle)) return color;
  }
  const palette = ['#C8B890', '#8A9C86', '#AA7088', '#D8A878', '#9A8AB8', '#7FA8A4', '#B7918F'];
  return palette[hashString(model) % palette.length];
}

function hexToRgb(hex: string): [number, number, number] | null {
  const normalized = hex.trim().replace(/^#/, '');
  if (!/^[0-9a-fA-F]{6}$/.test(normalized)) return null;
  return [
    Number.parseInt(normalized.slice(0, 2), 16),
    Number.parseInt(normalized.slice(2, 4), 16),
    Number.parseInt(normalized.slice(4, 6), 16),
  ];
}

function rgba(hex: string, alpha: number): string {
  const rgb = hexToRgb(hex);
  if (!rgb) return hex;
  return `rgba(${rgb[0]}, ${rgb[1]}, ${rgb[2]}, ${alpha})`;
}

function formatTokens(tokens: number): string {
  if (tokens >= 1_000) return `${(tokens / 1000).toFixed(1)}K tok`;
  return `${tokens} tok`;
}

function fitText(ctx: CanvasRenderingContext2D, text: string, maxWidth: number): string {
  if (ctx.measureText(text).width <= maxWidth) return text;

  let next = text;
  while (next.length > 4 && ctx.measureText(`${next}...`).width > maxWidth) {
    next = next.slice(0, -1);
  }
  return `${next}...`;
}

/** Animated horizontal cost race using Canvas 2D. */
export default function CostRace({ models, live = false, height = 260 }: CostRaceProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rafRef = useRef<number | null>(null);
  const valuesRef = useRef<Map<string, number>>(new Map());
  const [rows, setRows] = useState<CostRaceModel[]>([]);
  const { get, isLive } = useLiveApi();
  const { connected, lastEvent } = useBenchSSE({ enabled: live && isLive });

  const liveStatus = !live ? 'API' : isLive ? (connected ? 'LIVE' : 'CONNECTING') : 'OFFLINE';

  useEffect(() => {
    if (models == null) return;

    valuesRef.current = new Map();
    if (rafRef.current != null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
    setRows(sortRows(normalizeRows(models)));
  }, [models]);

  useEffect(() => {
    if (models != null) return;

    let cancelled = false;

    (async () => {
      try {
        const payload = await get<CostRaceResponse>('/api/bench/cost-summary');
        if (cancelled) return;

        const incoming = normalizeRows(payload);
        setRows((prev) => mergeRows(prev, incoming));
      } catch {
        if (cancelled) return;
        setRows([]);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [get, models]);

  useEffect(() => {
    if (!lastEvent || lastEvent.type !== 'BenchTaskCompleted') return;

    const result = lastEvent.result;
    const tokens = Math.max(0, result.tokens_in + result.tokens_out);

    setRows((prev) => {
      const map = new Map<string, CostRaceModel>();
      for (const row of prev) map.set(row.model, { ...row });

      const existing = map.get(result.model);
      map.set(result.model, existing
        ? {
            ...existing,
            cost_usd: existing.cost_usd + result.cost_usd,
            tokens: existing.tokens + tokens,
            tasks: existing.tasks + 1,
          }
        : {
            model: result.model,
            cost_usd: result.cost_usd,
            tokens,
            tasks: 1,
            color: getModelColor(result.model),
          });

      return sortRows([...map.values()]);
    });
  }, [lastEvent]);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    canvas.width = Math.max(1, rect.width * dpr);
    canvas.height = Math.max(1, rect.height * dpr);
    ctx.scale(dpr, dpr);

    const w = rect.width;
    const h = rect.height;
    const titleColor = '#8a7a88';
    const muted = '#6a5a68';
    const labelPad = Math.min(Math.max(w * 0.30, 112), 170);
    const valuePad = Math.min(Math.max(w * 0.32, 132), 180);
    const pad = { top: 36, right: valuePad, bottom: 18, left: labelPad };
    const plotW = Math.max(w - pad.left - pad.right, 8);

    ctx.clearRect(0, 0, w, h);

    ctx.fillStyle = titleColor;
    ctx.font = '11px "General Sans", sans-serif';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'alphabetic';
    ctx.fillText('MODEL COST RACE', pad.left, 16);

    if (rows.length === 0) {
      ctx.fillStyle = muted;
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.fillText('loading cost summary...', pad.left, 36);
      return;
    }

    const sorted = sortRows(rows);
    const maxCost = Math.max(...sorted.map((row) => row.cost_usd), 0.01);
    const rowGap = Math.max(6, Math.min(10, Math.round(h * 0.025)));
    const rowH = Math.min(
      34,
      Math.max((h - pad.top - pad.bottom - rowGap * (sorted.length - 1)) / sorted.length, 22),
    );
    const barH = Math.max(14, rowH - 6);
    const modelSet = new Set(sorted.map((row) => row.model));
    const displayValues = valuesRef.current;
    let needsNextFrame = false;

    for (const key of [...displayValues.keys()]) {
      if (!modelSet.has(key)) displayValues.delete(key);
    }

    ctx.strokeStyle = 'rgba(255,255,255,0.04)';
    ctx.lineWidth = 1;
    ctx.fillStyle = muted;
    ctx.font = '8px "JetBrains Mono", monospace';
    ctx.textAlign = 'center';
    for (let i = 0; i <= 4; i += 1) {
      const x = pad.left + (i / 4) * plotW;
      ctx.beginPath();
      ctx.moveTo(x, pad.top - 2);
      ctx.lineTo(x, h - pad.bottom + 4);
      ctx.stroke();
      ctx.fillText(`$${((maxCost * i) / 4).toFixed(2)}`, x, 26);
    }

    sorted.forEach((row, index) => {
      const y = pad.top + index * (rowH + rowGap);
      const centerY = y + rowH / 2;
      const current = displayValues.get(row.model) ?? 0;
      const next = current + (row.cost_usd - current) * 0.16;
      displayValues.set(row.model, next);

      if (Math.abs(next - row.cost_usd) > 0.0005) needsNextFrame = true;

      const color = getModelColor(row.model, row.color);
      const labelColor = index === 0 ? '#c4b4c4' : '#8a7a88';
      const valueX = pad.left + plotW + 10;
      const maxLabelWidth = Math.max(pad.left - 18, 40);
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.font = '10px "JetBrains Mono", monospace';
      const modelLabel = fitText(ctx, row.model, maxLabelWidth);
      const barW = Math.max((next / maxCost) * plotW, next > 0 ? 2 : 0);

      ctx.fillStyle = labelColor;
      ctx.fillText(modelLabel, pad.left - 10, centerY);

      ctx.fillStyle = 'rgba(255,255,255,0.03)';
      ctx.beginPath();
      ctx.roundRect(pad.left, y + 2, plotW, barH, 4);
      ctx.fill();

      if (barW > 0) {
        const tipColor = getModelColor(row.model, row.color);
        const rgb = hexToRgb(tipColor);
        const fillColor = rgb ? rgba(tipColor, 1) : tipColor;
        const barGradient = ctx.createLinearGradient(pad.left, 0, pad.left + barW, 0);
        if (rgb) {
          barGradient.addColorStop(0, rgba(tipColor, 0.38));
          barGradient.addColorStop(1, rgba(tipColor, 0.88));
        } else {
          barGradient.addColorStop(0, fillColor);
          barGradient.addColorStop(1, fillColor);
        }

        ctx.fillStyle = barGradient;
        ctx.beginPath();
        ctx.roundRect(pad.left, y + 2, barW, barH, 4);
        ctx.fill();

        if (rgb) {
          const glow = ctx.createRadialGradient(
            pad.left + barW,
            centerY,
            0,
            pad.left + barW,
            centerY,
            Math.max(barH * 0.9, 12),
          );
          glow.addColorStop(0, rgba(tipColor, 0.28));
          glow.addColorStop(1, rgba(tipColor, 0));
          ctx.fillStyle = glow;
          ctx.beginPath();
          ctx.arc(pad.left + barW, centerY, Math.max(barH * 0.7, 8), 0, Math.PI * 2);
          ctx.fill();
        }
      }

      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.font = 'bold 10px "JetBrains Mono", monospace';
      ctx.fillStyle = color;
      ctx.fillText(`$${row.cost_usd.toFixed(3)}`, valueX, centerY - 5);

      ctx.font = '8px "JetBrains Mono", monospace';
      ctx.fillStyle = muted;
      const taskLabel = `${row.tasks} task${row.tasks === 1 ? '' : 's'}`;
      ctx.fillText(`${formatTokens(row.tokens)}  ${taskLabel}`, valueX, centerY + 7);
    });

    if (needsNextFrame) {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
      rafRef.current = requestAnimationFrame(() => {
        rafRef.current = null;
        draw();
      });
    } else if (rafRef.current != null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
  }, [rows]);

  useEffect(() => {
    if (rafRef.current != null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }

    draw();

    const ro = new ResizeObserver(draw);
    if (canvasRef.current) ro.observe(canvasRef.current);

    return () => {
      if (rafRef.current != null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
      ro.disconnect();
    };
  }, [draw]);

  return (
    <div
      style={{
        position: 'relative',
        width: '100%',
        height,
        overflow: 'hidden',
        borderRadius: 12,
        border: '1px solid rgba(255,255,255,0.04)',
        background: 'linear-gradient(180deg, rgba(255,255,255,0.02), rgba(255,255,255,0.01))',
      }}
    >
      <div
        style={{
          position: 'absolute',
          top: 10,
          right: 12,
          zIndex: 1,
          padding: '2px 8px',
          borderRadius: 999,
          border: '1px solid rgba(255,255,255,0.08)',
          background: 'rgba(16, 16, 18, 0.55)',
          color: liveStatus === 'LIVE' ? 'var(--success)' : 'var(--text-soft)',
          fontFamily: 'var(--mono)',
          fontSize: 13,
          letterSpacing: '0.08em',
          pointerEvents: 'none',
          textTransform: 'uppercase',
        }}
      >
        {liveStatus}
      </div>
      <canvas
        ref={canvasRef}
        style={{
          width: '100%',
          height: '100%',
          display: 'block',
        }}
      />
    </div>
  );
}
