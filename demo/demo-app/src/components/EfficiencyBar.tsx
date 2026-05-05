import { useEffect, useMemo, useRef, useState, useCallback } from 'react';
import Mosaic, { MosaicCell } from './Mosaic';
import './EfficiencyBar.css';

/* ── Types ───────────────────────────────────────────────── */

export interface EfficiencySegment {
  label: string;
  value: number;
  color?: string;
}

export interface EfficiencyThreshold {
  value: number;
  label?: string;
}

export interface EfficiencyMetric {
  label: string;
  value: number;
  format?: (n: number) => string;
  color?: 'rose' | 'bone' | 'dream' | 'success' | 'warning';
  /** Max value for bar fill (defaults to 100). */
  max?: number;
  /** Cost segments that compose this value. */
  segments?: EfficiencySegment[];
  /** Threshold tick marks along the bar. */
  thresholds?: EfficiencyThreshold[];
  /** Previous value for comparison mode — shows delta region. */
  previousValue?: number;
}

interface EfficiencyBarProps {
  metrics: EfficiencyMetric[];
}

/* ── Helpers ─────────────────────────────────────────────── */

const COLOR_MAP: Record<string, string> = {
  rose: 'var(--rose-bright)',
  bone: 'var(--bone-bright)',
  dream: 'var(--dream-bright)',
  success: 'var(--success)',
  warning: 'var(--warning)',
};

const SEGMENT_COLORS = [
  'var(--rose-bright)',
  'var(--dream-bright)',
  'var(--bone-bright)',
  'var(--success)',
  'var(--warning)',
  'var(--lane-sage)',
  'var(--lane-clay)',
];

function efficiencyGradientColor(ratio: number): string {
  // green (efficient) -> amber -> red (inefficient)
  if (ratio <= 0.4) return 'var(--success)';
  if (ratio <= 0.7) return 'var(--warning)';
  return 'var(--status-error)';
}

function useSpringValue(target: number, tension = 0.08): number {
  const [current, setCurrent] = useState(0);
  const rafRef = useRef<number | null>(null);
  const currentRef = useRef(0);
  const targetRef = useRef(target);

  targetRef.current = target;

  useEffect(() => {
    const prefersReduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (prefersReduced) {
      setCurrent(target);
      currentRef.current = target;
      return;
    }

    function tick() {
      const diff = targetRef.current - currentRef.current;
      if (Math.abs(diff) < 0.01) {
        currentRef.current = targetRef.current;
        setCurrent(targetRef.current);
        rafRef.current = null;
        return;
      }
      currentRef.current += diff * tension;
      setCurrent(currentRef.current);
      rafRef.current = requestAnimationFrame(tick);
    }

    if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(tick);

    return () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
    };
  }, [target, tension]);

  return current;
}

/* ── AnimatedValue: counts up to target ──────────────────── */

function AnimatedValue({ value, format }: { value: number; format?: (n: number) => string }) {
  const animated = useSpringValue(value);
  const display = format ? format(animated) : String(Math.round(animated));
  return <span>{display}</span>;
}

/* ── FillBar: animated bar with shimmer + segments ───────── */

interface FillBarProps {
  metric: EfficiencyMetric;
  index: number;
}

function FillBar({ metric, index }: FillBarProps) {
  const max = metric.max ?? 100;
  const ratio = max > 0 ? Math.min(metric.value / max, 1) : 0;
  const animatedRatio = useSpringValue(ratio, 0.06);
  const baseColor = metric.color ? COLOR_MAP[metric.color] ?? 'var(--rose-bright)' : efficiencyGradientColor(ratio);
  const [hovered, setHovered] = useState(false);
  const [pulse, setPulse] = useState(false);
  const prevValueRef = useRef(metric.value);

  // Pulse on change
  useEffect(() => {
    if (metric.value !== prevValueRef.current) {
      prevValueRef.current = metric.value;
      setPulse(true);
      const t = setTimeout(() => setPulse(false), 600);
      return () => clearTimeout(t);
    }
  }, [metric.value]);

  const segments = metric.segments;
  const hasSegments = segments && segments.length > 0;
  const segmentTotal = hasSegments ? segments.reduce((s, seg) => s + seg.value, 0) : 0;

  // Comparison mode
  const hasPrev = metric.previousValue != null && metric.previousValue !== metric.value;
  const prevRatio = hasPrev && max > 0 ? Math.min(metric.previousValue! / max, 1) : 0;
  const animatedPrevRatio = useSpringValue(hasPrev ? prevRatio : 0, 0.06);

  const onEnter = useCallback(() => setHovered(true), []);
  const onLeave = useCallback(() => setHovered(false), []);

  return (
    <div
      className={`ebar-fill-wrap${hovered ? ' ebar-hovered' : ''}${pulse ? ' ebar-pulse' : ''}`}
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
      style={{ animationDelay: `${index * 80}ms` }}
    >
      {/* Track */}
      <div className="ebar-track">
        {/* Comparison: previous value ghost */}
        {hasPrev && (
          <div
            className="ebar-prev-fill"
            style={{
              width: `${animatedPrevRatio * 100}%`,
            }}
          />
        )}

        {/* Main fill or segments */}
        {hasSegments ? (
          <div className="ebar-segments" style={{ width: `${animatedRatio * 100}%` }}>
            {segments.map((seg, si) => {
              const segRatio = segmentTotal > 0 ? seg.value / segmentTotal : 0;
              return (
                <div
                  key={seg.label}
                  className="ebar-segment"
                  style={{
                    width: `${segRatio * 100}%`,
                    background: seg.color ?? SEGMENT_COLORS[si % SEGMENT_COLORS.length],
                    animationDelay: `${index * 80 + si * 120}ms`,
                  }}
                />
              );
            })}
          </div>
        ) : (
          <div
            className="ebar-fill"
            style={{
              width: `${animatedRatio * 100}%`,
              background: baseColor,
            }}
          >
            <div className="ebar-shimmer" />
          </div>
        )}

        {/* Comparison delta highlight */}
        {hasPrev && (
          <div
            className={`ebar-delta ${metric.value > metric.previousValue! ? 'ebar-delta-up' : 'ebar-delta-down'}`}
            style={{
              left: `${Math.min(animatedPrevRatio, animatedRatio) * 100}%`,
              width: `${Math.abs(animatedRatio - animatedPrevRatio) * 100}%`,
            }}
          />
        )}

        {/* Threshold markers */}
        {metric.thresholds?.map((th) => {
          const thRatio = max > 0 ? Math.min(th.value / max, 1) : 0;
          return (
            <div
              key={th.value}
              className="ebar-threshold"
              style={{ left: `${thRatio * 100}%` }}
            >
              <div className="ebar-threshold-tick" />
              {th.label && <span className="ebar-threshold-label">{th.label}</span>}
            </div>
          );
        })}
      </div>

      {/* Hover breakdown for segments */}
      {hasSegments && hovered && (
        <div className="ebar-breakdown">
          {segments.map((seg, si) => {
            const segRatio = segmentTotal > 0 ? seg.value / segmentTotal : 0;
            return (
              <div key={seg.label} className="ebar-breakdown-row" style={{ animationDelay: `${si * 60}ms` }}>
                <span
                  className="ebar-breakdown-swatch"
                  style={{ background: seg.color ?? SEGMENT_COLORS[si % SEGMENT_COLORS.length] }}
                />
                <span className="ebar-breakdown-label">{seg.label}</span>
                <span className="ebar-breakdown-value">
                  {metric.format ? metric.format(seg.value) : seg.value.toFixed(1)}
                </span>
                <div className="ebar-breakdown-bar">
                  <div
                    className="ebar-breakdown-bar-fill"
                    style={{
                      width: `${segRatio * 100}%`,
                      background: seg.color ?? SEGMENT_COLORS[si % SEGMENT_COLORS.length],
                    }}
                  />
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

/* ── EfficiencyBar ───────────────────────────────────────── */

export default function EfficiencyBar({ metrics }: EfficiencyBarProps) {
  const cols = Math.min(Math.max(metrics.length, 2), 6) as 2 | 3 | 4 | 5 | 6;
  const hasAnyBar = useMemo(
    () => metrics.some((m) => m.max != null || m.segments != null || m.thresholds != null || m.previousValue != null),
    [metrics],
  );

  return (
    <div className="efficiency-bar">
      <Mosaic columns={cols}>
        {metrics.map((m) => (
          <MosaicCell
            key={m.label}
            label={m.label}
            value={<AnimatedValue value={m.value} format={m.format} />}
            color={m.color}
            mono
          />
        ))}
      </Mosaic>

      {/* Fill bars rendered below the mosaic grid */}
      {hasAnyBar && (
        <div className="ebar-bars" style={{ gridTemplateColumns: `repeat(${cols}, 1fr)` }}>
          {metrics.map((m, i) => (
            <FillBar key={m.label} metric={m} index={i} />
          ))}
        </div>
      )}
    </div>
  );
}
