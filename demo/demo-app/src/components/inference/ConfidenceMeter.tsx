import { useRef, useEffect, useState } from 'react';
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

/** Determine fill gradient class from confidence level. */
function fillClass(c: number): string {
  if (c < 0.3) return 'confidence-meter__fill--dim';
  if (c < 0.6) return 'confidence-meter__fill--building';
  if (c < 0.8) return 'confidence-meter__fill--confident';
  return 'confidence-meter__fill--prismatic';
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
  const pct = Math.round(clamped * 100);
  const prevConfidence = useRef(clamped);
  const [crystallizing, setCrystallizing] = useState(false);

  // Detect crossing the 0.8 crystallization threshold
  useEffect(() => {
    const prev = prevConfidence.current;
    prevConfidence.current = clamped;

    if (prev < 0.8 && clamped >= 0.8) {
      setCrystallizing(true);
      const timer = setTimeout(() => setCrystallizing(false), 600);
      return () => clearTimeout(timer);
    }
  }, [clamped]);

  const trendSymbol = TREND_SYMBOL[trend];
  const trendLabel = TREND_LABEL[trend];

  if (compact) {
    return (
      <div className={`confidence-meter confidence-meter--compact${className ? ` ${className}` : ''}`}>
        <div className="confidence-meter__header">
          <span className="confidence-meter__label">{label}</span>
        </div>
        <div
          className={`confidence-meter__track${crystallizing ? ' confidence-meter__track--crystallize' : ''}`}
        >
          <div
            className={`confidence-meter__fill ${fillClass(clamped)}`}
            style={{ width: `${pct}%` }}
          />
        </div>
        <div className="confidence-meter__inline-stats">
          <span>{pct}%</span>
          <span className={`confidence-meter__trend confidence-meter__trend--${trend}`}>
            {trendSymbol}
          </span>
        </div>
      </div>
    );
  }

  return (
    <div className={`confidence-meter${className ? ` ${className}` : ''}`}>
      <div className="confidence-meter__header">
        <span className="confidence-meter__label">{label}</span>
        <span className="confidence-meter__decisions">{decisions} decisions</span>
      </div>

      <div
        className={`confidence-meter__track${crystallizing ? ' confidence-meter__track--crystallize' : ''}`}
      >
        <div
          className={`confidence-meter__fill ${fillClass(clamped)}`}
          style={{ width: `${pct}%` }}
        />
      </div>

      <div className="confidence-meter__footer">
        <span className="confidence-meter__pct">{pct}%</span>
        <span className={`confidence-meter__trend confidence-meter__trend--${trend}`}>
          {trendSymbol} {trendLabel}
        </span>
      </div>
    </div>
  );
}
