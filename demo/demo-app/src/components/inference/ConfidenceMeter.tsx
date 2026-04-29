import { useRef, useEffect, useState, useCallback } from 'react';
import './ConfidenceMeter.css';

interface ConfidenceMeterProps {
  confidence: number;
  trend: 'improving' | 'stable' | 'declining';
  decisions: number;
  label?: string;
  compact?: boolean;
  className?: string;
}

const TREND_SYMBOL: Record<ConfidenceMeterProps['trend'], string> = {
  improving: '\u25b2',
  stable: '\u2014',
  declining: '\u25bc',
};

const TREND_LABEL: Record<ConfidenceMeterProps['trend'], string> = {
  improving: 'IMPROVING',
  stable: 'STABLE',
  declining: 'DECLINING',
};

/** easeOutExpo curve for animated transitions. */
function easeOutExpo(t: number): number {
  return t >= 1 ? 1 : 1 - Math.pow(2, -10 * t);
}

/** Interpolate color between red, amber, green based on confidence 0-1. */
function confidenceColor(c: number): string {
  if (c < 0.3) {
    // red zone: status-error (#fb7185)
    return 'var(--status-error)';
  }
  if (c < 0.7) {
    // amber zone: status-warning (#fbbf24)
    return 'var(--status-warning)';
  }
  // green zone: status-success (#4ade80)
  return 'var(--status-success)';
}

/** Get an RGB string for the glow effect at a given confidence level. */
function glowRgb(c: number): string {
  if (c < 0.3) return '251, 113, 133';     // red
  if (c < 0.7) return '251, 191, 36';      // amber
  return '74, 222, 128';                     // green
}

/** Determine fill gradient class from confidence level. */
function fillClass(c: number): string {
  if (c < 0.3) return 'confidence-meter__fill--dim';
  if (c < 0.6) return 'confidence-meter__fill--building';
  if (c < 0.8) return 'confidence-meter__fill--confident';
  return 'confidence-meter__fill--prismatic';
}

/** Hook: animate a number from prev to target using easeOutExpo over given duration. */
function useAnimatedValue(target: number, duration: number): number {
  const [display, setDisplay] = useState(target);
  const rafRef = useRef<number>(0);
  const startRef = useRef<{ from: number; to: number; startTime: number } | null>(null);

  useEffect(() => {
    const from = display;
    if (from === target) return;
    const startTime = performance.now();
    startRef.current = { from, to: target, startTime };

    function tick(now: number) {
      if (!startRef.current) return;
      const elapsed = now - startRef.current.startTime;
      const progress = Math.min(elapsed / duration, 1);
      const eased = easeOutExpo(progress);
      const current = startRef.current.from + (startRef.current.to - startRef.current.from) * eased;
      setDisplay(current);
      if (progress < 1) {
        rafRef.current = requestAnimationFrame(tick);
      }
    }

    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target, duration]);

  return display;
}

// SVG arc geometry for the gauge
const ARC_CX = 60;
const ARC_CY = 52;
const ARC_R = 42;
const ARC_START_ANGLE = -210; // degrees from 12-o'clock
const ARC_END_ANGLE = 30;
const ARC_SWEEP = ARC_END_ANGLE - ARC_START_ANGLE; // 240 degrees

/** Convert angle (degrees, 0 = top, CW positive) to SVG x,y. */
function polarToSvg(cx: number, cy: number, r: number, angleDeg: number): [number, number] {
  // SVG: 0deg = right, so subtract 90 to make 0deg = top
  const rad = ((angleDeg - 90) * Math.PI) / 180;
  return [cx + r * Math.cos(rad), cy + r * Math.sin(rad)];
}

/** Create an SVG arc path from startAngle to endAngle. */
function arcPath(cx: number, cy: number, r: number, startDeg: number, endDeg: number): string {
  const [sx, sy] = polarToSvg(cx, cy, r, startDeg);
  const [ex, ey] = polarToSvg(cx, cy, r, endDeg);
  const sweep = endDeg - startDeg;
  const largeArc = Math.abs(sweep) > 180 ? 1 : 0;
  const dir = sweep > 0 ? 1 : 0;
  return `M ${sx} ${sy} A ${r} ${r} 0 ${largeArc} ${dir} ${ex} ${ey}`;
}

export default function ConfidenceMeter({
  confidence,
  trend,
  decisions,
  label = 'CASCADE CONFIDENCE',
  compact = false,
  className,
}: ConfidenceMeterProps) {
  const clamped = Math.max(0, Math.min(1, confidence));
  const prevConfidence = useRef(clamped);
  const [crystallizing, setCrystallizing] = useState(false);
  const [flashing, setFlashing] = useState(false);

  // Animated value for the gauge
  const animatedValue = useAnimatedValue(clamped, 800);
  const animatedPct = Math.round(animatedValue * 100);

  // Detect changes for flash effect
  const handleChange = useCallback((prev: number, next: number) => {
    if (Math.abs(prev - next) > 0.01) {
      setFlashing(true);
      setTimeout(() => setFlashing(false), 300);
    }
    if (prev < 0.8 && next >= 0.8) {
      setCrystallizing(true);
      setTimeout(() => setCrystallizing(false), 600);
    }
  }, []);

  useEffect(() => {
    const prev = prevConfidence.current;
    prevConfidence.current = clamped;
    handleChange(prev, clamped);
  }, [clamped, handleChange]);

  const trendSymbol = TREND_SYMBOL[trend];
  const trendLabel = TREND_LABEL[trend];
  const color = confidenceColor(animatedValue);
  const glow = glowRgb(animatedValue);

  // Arc gauge SVG
  const fillAngle = ARC_START_ANGLE + ARC_SWEEP * animatedValue;
  const trackPath = arcPath(ARC_CX, ARC_CY, ARC_R, ARC_START_ANGLE, ARC_END_ANGLE);
  const fillPath = animatedValue > 0.005
    ? arcPath(ARC_CX, ARC_CY, ARC_R, ARC_START_ANGLE, fillAngle)
    : '';

  // Glow dot at the end of the fill arc
  const [dotX, dotY] = polarToSvg(ARC_CX, ARC_CY, ARC_R, fillAngle);

  if (compact) {
    const pct = Math.round(clamped * 100);
    return (
      <div className={`confidence-meter confidence-meter--compact${className ? ` ${className}` : ''}`}>
        <div className="confidence-meter__header">
          <span className="confidence-meter__label">{label}</span>
        </div>
        <div
          className={`confidence-meter__track${crystallizing ? ' confidence-meter__track--crystallize' : ''}${flashing ? ' confidence-meter__track--flash' : ''}`}
        >
          <div
            className={`confidence-meter__fill ${fillClass(clamped)}`}
            style={{ width: `${pct}%` }}
          />
          {/* Glow dot at the tip of the fill */}
          <div
            className="confidence-meter__glow-dot"
            style={{
              left: `${pct}%`,
              background: color,
              boxShadow: `0 0 6px rgba(${glow}, 0.8), 0 0 12px rgba(${glow}, 0.4)`,
            }}
          />
        </div>
        <div className="confidence-meter__inline-stats">
          <span className={`confidence-meter__counter${flashing ? ' confidence-meter__counter--flash' : ''}`}>
            {animatedPct}%
          </span>
          <span className={`confidence-meter__trend confidence-meter__trend--${trend}`}>
            {trendSymbol}
          </span>
        </div>
      </div>
    );
  }

  return (
    <div className={`confidence-meter confidence-meter--full${className ? ` ${className}` : ''}${flashing ? ' confidence-meter--flash' : ''}`}>
      <div className="confidence-meter__header">
        <span className="confidence-meter__label">{label}</span>
        <span className="confidence-meter__decisions">{decisions} decisions</span>
      </div>

      {/* SVG arc gauge */}
      <div className={`confidence-meter__gauge${crystallizing ? ' confidence-meter__gauge--crystallize' : ''}`}>
        <svg
          viewBox="0 0 120 72"
          className="confidence-meter__svg"
          aria-hidden="true"
        >
          {/* Glow filter */}
          <defs>
            <filter id="cm-glow" x="-50%" y="-50%" width="200%" height="200%">
              <feGaussianBlur in="SourceGraphic" stdDeviation="2" result="blur" />
              <feMerge>
                <feMergeNode in="blur" />
                <feMergeNode in="SourceGraphic" />
              </feMerge>
            </filter>
          </defs>

          {/* Background track */}
          <path
            d={trackPath}
            fill="none"
            stroke="var(--border-soft)"
            strokeWidth="4"
            strokeLinecap="round"
          />

          {/* Filled arc */}
          {fillPath && (
            <path
              d={fillPath}
              fill="none"
              stroke={color}
              strokeWidth="4"
              strokeLinecap="round"
              filter="url(#cm-glow)"
              className="confidence-meter__arc-fill"
            />
          )}

          {/* Pulsing glow dot at current value */}
          {animatedValue > 0.005 && (
            <circle
              cx={dotX}
              cy={dotY}
              r="4"
              fill={color}
              className="confidence-meter__pulse-dot"
              style={{ filter: `drop-shadow(0 0 4px rgba(${glow}, 0.8))` }}
            />
          )}
        </svg>

        {/* Center readout */}
        <div className="confidence-meter__readout">
          <span
            className={`confidence-meter__value${flashing ? ' confidence-meter__value--flash' : ''}`}
            style={{ color }}
          >
            {animatedPct}
          </span>
          <span className="confidence-meter__unit">%</span>
        </div>
      </div>

      <div className="confidence-meter__footer">
        <span className={`confidence-meter__trend confidence-meter__trend--${trend}`}>
          {trendSymbol} {trendLabel}
        </span>
      </div>
    </div>
  );
}
