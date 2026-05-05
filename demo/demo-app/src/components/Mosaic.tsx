import { type ReactNode, useEffect, useRef, useState, useCallback, useMemo, Children } from 'react';
import FlatIcon, { inferIcon, type FlatIconName } from './FlatIcon';

/* ── useCountUp ────────────────────────────────────────── */

function easeOutExpo(t: number): number {
  return t === 1 ? 1 : 1 - Math.pow(2, -10 * t);
}

function useCountUp(target: number, duration = 800): number {
  const [current, setCurrent] = useState(0);
  const startRef = useRef<number | null>(null);
  const fromRef = useRef(0);
  const rafRef = useRef(0);

  useEffect(() => {
    fromRef.current = current;
    startRef.current = null;
    cancelAnimationFrame(rafRef.current);

    const animate = (ts: number) => {
      if (startRef.current === null) startRef.current = ts;
      const elapsed = ts - startRef.current;
      const progress = Math.min(elapsed / duration, 1);
      const eased = easeOutExpo(progress);
      const val = fromRef.current + (target - fromRef.current) * eased;
      setCurrent(val);
      if (progress < 1) rafRef.current = requestAnimationFrame(animate);
    };

    rafRef.current = requestAnimationFrame(animate);
    return () => cancelAnimationFrame(rafRef.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target, duration]);

  return current;
}

/* ── usePrevious ───────────────────────────────────────── */

function usePrevious<T>(value: T): T | undefined {
  const ref = useRef<T | undefined>(undefined);
  useEffect(() => { ref.current = value; });
  return ref.current;
}

/* ── MiniSparkline ─────────────────────────────────────── */

interface SparklineProps {
  data: number[];
  color?: string;
  width?: number;
  height?: number;
}

function MiniSparkline({ data, color = 'var(--rose-glow)', width = 60, height = 20 }: SparklineProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const animRef = useRef(0);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || data.length < 2) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = width * dpr;
    canvas.height = height * dpr;
    ctx.scale(dpr, dpr);

    const min = Math.min(...data);
    const max = Math.max(...data);
    const range = max - min || 1;
    const stepX = width / (data.length - 1);

    const resolvedColor = (() => {
      if (color.startsWith('var(')) {
        const style = getComputedStyle(canvas);
        const varName = color.slice(4, -1);
        return style.getPropertyValue(varName).trim() || '#e8b5ce';
      }
      return color;
    })();

    let progress = 0;
    const duration = 600;
    let startTs: number | null = null;
    cancelAnimationFrame(animRef.current);

    const draw = (ts: number) => {
      if (startTs === null) startTs = ts;
      progress = Math.min((ts - startTs) / duration, 1);
      const eased = easeOutExpo(progress);
      const drawLen = Math.floor(eased * (data.length - 1)) + 1;

      ctx.clearRect(0, 0, width, height);
      ctx.beginPath();
      ctx.strokeStyle = resolvedColor;
      ctx.lineWidth = 1.5;
      ctx.lineJoin = 'round';
      ctx.lineCap = 'round';

      for (let i = 0; i < drawLen; i++) {
        const x = i * stepX;
        const y = height - ((data[i] - min) / range) * (height - 4) - 2;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.stroke();

      // gradient fill below line
      const grad = ctx.createLinearGradient(0, 0, 0, height);
      grad.addColorStop(0, resolvedColor + '30');
      grad.addColorStop(1, resolvedColor + '00');
      ctx.lineTo((drawLen - 1) * stepX, height);
      ctx.lineTo(0, height);
      ctx.closePath();
      ctx.fillStyle = grad;
      ctx.fill();

      if (progress < 1) animRef.current = requestAnimationFrame(draw);
    };

    animRef.current = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(animRef.current);
  }, [data, color, width, height]);

  return (
    <canvas
      ref={canvasRef}
      className="mosaic-sparkline"
      style={{ width, height, display: 'block', marginTop: 4 }}
    />
  );
}

/* ── MosaicCell ─────────────────────────────────────────── */

/** Color variants for mosaic cells */
export type MosaicColor = 'rose' | 'bone' | 'dream' | 'success' | 'warning' | 'teal' | 'amber' | 'violet';

interface MosaicCellProps {
  label: string;
  value: ReactNode;
  sub?: string;
  color?: MosaicColor;
  mono?: boolean;
  icon?: FlatIconName;
  /** Trend direction for status coloring */
  trend?: 'up' | 'down' | 'neutral';
  /** Sparkline data points */
  sparkline?: number[];
  /** Numeric target for animated count-up (use instead of value for numbers) */
  animateNumber?: number;
  /** Format function for animated number display */
  formatNumber?: (n: number) => string;
  /** Tooltip text shown on hover expand */
  tooltip?: string;
  /** Expanded detail shown on hover (e.g. cost breakdown, token split) */
  detail?: ReactNode;
}

const defaultFormat = (n: number) =>
  Number.isInteger(n) ? n.toLocaleString() : n.toFixed(2);

/** Map color names to CSS variable values */
const COLOR_MAP: Record<MosaicColor, string> = {
  rose: 'var(--rose-glow)',
  bone: 'var(--bone-bright)',
  dream: 'var(--dream-bright)',
  success: 'var(--success)',
  warning: 'var(--warning)',
  teal: 'var(--status-active)',
  amber: 'var(--status-warning)',
  violet: 'var(--status-blocked)',
};

/** Map color names to FlatIcon tone (fall back to closest match for new variants) */
const TONE_MAP: Record<MosaicColor, 'rose' | 'bone' | 'dream' | 'success' | 'warning' | 'muted'> = {
  rose: 'rose',
  bone: 'bone',
  dream: 'dream',
  success: 'success',
  warning: 'warning',
  teal: 'success',
  amber: 'warning',
  violet: 'dream',
};

export function MosaicCell({
  label, value, sub, color, mono, icon,
  trend, sparkline, animateNumber, formatNumber, tooltip, detail,
}: MosaicCellProps) {
  const cellRef = useRef<HTMLDivElement>(null);
  const [flash, setFlash] = useState(false);
  const [transitioning, setTransitioning] = useState(false);
  const [hovered, setHovered] = useState(false);

  // Animated number
  const animatedNum = useCountUp(animateNumber ?? 0, 800);
  const fmt = formatNumber ?? defaultFormat;

  const displayValue = animateNumber !== undefined ? fmt(animatedNum) : value;

  // Serialize display value for transition detection
  const serializedValue = typeof displayValue === 'string' || typeof displayValue === 'number'
    ? String(displayValue)
    : null;
  const prevSerializedValue = usePrevious(serializedValue);

  // Value transition: slide old out, slide new in
  useEffect(() => {
    if (
      serializedValue !== null &&
      prevSerializedValue !== null &&
      serializedValue !== prevSerializedValue
    ) {
      setFlash(true);
      setTransitioning(true);
      const flashTimer = setTimeout(() => setFlash(false), 400);
      const transTimer = setTimeout(() => setTransitioning(false), 350);
      return () => { clearTimeout(flashTimer); clearTimeout(transTimer); };
    }
  }, [serializedValue, prevSerializedValue]);

  // Also flash on animateNumber change
  const prevAnimNum = usePrevious(animateNumber);
  useEffect(() => {
    if (animateNumber !== undefined && prevAnimNum !== undefined
        && animateNumber !== prevAnimNum) {
      setFlash(true);
      const timer = setTimeout(() => setFlash(false), 400);
      return () => clearTimeout(timer);
    }
  }, [animateNumber, prevAnimNum]);

  // Determine trend color override
  const trendColor = trend === 'up'
    ? 'var(--success)'
    : trend === 'down'
      ? 'var(--warning)'
      : undefined;

  const resolvedColor = color ?? 'rose';
  const colorVar = trendColor ?? COLOR_MAP[resolvedColor];
  const tone = TONE_MAP[resolvedColor];

  // Sparkline color
  const sparkColor = useMemo(() => {
    if (trend === 'up') return 'var(--success)';
    if (trend === 'down') return 'var(--warning)';
    return colorVar;
  }, [trend, colorVar]);

  const onEnter = useCallback(() => setHovered(true), []);
  const onLeave = useCallback(() => setHovered(false), []);

  const trendIndicator = trend === 'up' ? '\u25B2'
    : trend === 'down' ? '\u25BC'
      : null;

  return (
    <div
      ref={cellRef}
      className={`cell${flash ? ' cell-flash' : ''}${hovered ? ' cell-hovered' : ''}`}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
    >
      <div className="k mosaic-label-stagger">
        <FlatIcon name={icon ?? inferIcon(label)} size={14} tone={tone} className="mosaic-icon mosaic-icon-bounce" />
        <span>{label}</span>
        {trendIndicator && (
          <span className="mosaic-trend" style={{ color: trendColor }}>
            {trendIndicator}
          </span>
        )}
      </div>
      <div className={`v${mono ? ' mono' : ''}${transitioning ? ' v-transition' : ''}`} style={{ color: colorVar }}>
        {displayValue}
      </div>
      {sparkline && sparkline.length >= 2 && (
        <MiniSparkline data={sparkline} color={sparkColor} />
      )}
      {sub && <div className="sub">{sub}</div>}
      {tooltip && hovered && (
        <div className="mosaic-tooltip">{tooltip}</div>
      )}
      {detail && hovered && (
        <div className="mosaic-detail">{detail}</div>
      )}
    </div>
  );
}

/* ── Mosaic ──────────────────────────────────────────────── */

interface MosaicProps {
  columns: 2 | 3 | 4 | 5 | 6;
  children: ReactNode;
  className?: string;
  style?: React.CSSProperties;
  /** Message shown when all cells have '--' values. Defaults to "Waiting for data..." */
  emptyMessage?: string;
}

/**
 * Check if all MosaicCell children have empty/placeholder values.
 * Looks for children with value prop of '--' or '-' or '...' or '0'.
 */
function isAllEmpty(children: ReactNode): boolean {
  const arr = Children.toArray(children);
  if (arr.length === 0) return true;
  return arr.every((child) => {
    if (typeof child === 'object' && child !== null && 'props' in child) {
      const v = (child as { props: { value?: ReactNode } }).props.value;
      if (typeof v === 'string') return v === '--' || v === '-' || v === '...';
    }
    return false;
  });
}

/**
 * Grid of metric cells with 1px gap (gap IS the border color).
 * Cells have bg-void background. Uses canonical .mosaic from rosedust.css.
 */
export default function Mosaic({ columns, children, className, style, emptyMessage }: MosaicProps) {
  const empty = isAllEmpty(children);

  return (
    <div
      className={`mosaic${className ? ` ${className}` : ''}${empty ? ' mosaic-empty' : ''}`}
      style={{
        gridTemplateColumns: `repeat(${columns}, 1fr)`,
        ...style,
      }}
    >
      {children}
      {empty && (
        <div className="mosaic-empty-overlay">
          <span className="mosaic-empty-text">{emptyMessage ?? 'Waiting for data\u2026'}</span>
        </div>
      )}
    </div>
  );
}
