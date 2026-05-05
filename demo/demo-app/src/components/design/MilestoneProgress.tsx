import { useRef, useEffect, useState, type ReactNode } from 'react';
import './MilestoneProgress.css';

interface Milestone {
  position: number;
  label: string;
  icon?: ReactNode;
  reached?: boolean;
}

interface MilestoneProgressProps {
  value: number;
  milestones: Milestone[];
  height?: number;
  showLabels?: boolean;
  className?: string;
}

export function MilestoneProgress({
  value,
  milestones,
  height = 6,
  showLabels = true,
  className,
}: MilestoneProgressProps) {
  const clamped = Math.max(0, Math.min(100, value));
  const prevValue = useRef(clamped);
  const [justCrossed, setJustCrossed] = useState<Set<number>>(new Set());

  useEffect(() => {
    const prev = prevValue.current;
    prevValue.current = clamped;

    // Detect milestones just crossed
    const crossed = new Set<number>();
    for (let i = 0; i < milestones.length; i++) {
      const pos = milestones[i].position;
      if (prev < pos && clamped >= pos) {
        crossed.add(i);
      }
    }

    if (crossed.size > 0) {
      setJustCrossed(crossed);
      const timer = setTimeout(() => setJustCrossed(new Set()), 700);
      return () => clearTimeout(timer);
    }
  }, [clamped, milestones]);

  return (
    <div className={['milestone-progress', className].filter(Boolean).join(' ')}>
      {/* Track + fill */}
      <div className="milestone-progress__track" style={{ height }}>
        <div
          className="milestone-progress__fill"
          style={{ width: `${clamped}%` }}
        />

        {/* Marker layer */}
        <div className="milestone-progress__markers">
          {milestones.map((ms, i) => {
            const reached = ms.reached ?? clamped >= ms.position;
            const sparkle = justCrossed.has(i);

            return (
              <div key={i} style={{ position: 'absolute', left: `${ms.position}%`, top: '50%' }}>
                <div
                  className={[
                    'milestone-progress__marker',
                    reached ? 'milestone-progress__marker--reached' : '',
                  ]
                    .filter(Boolean)
                    .join(' ')}
                  style={{ position: 'relative', top: 0, left: 0, transform: 'translate(-50%, -50%) rotate(45deg)' }}
                >
                  {ms.icon && (
                    <span className="milestone-progress__marker-icon">
                      {ms.icon}
                    </span>
                  )}
                </div>

                {/* Sparkle effect */}
                {sparkle && (
                  <span
                    className="milestone-progress__sparkle milestone-progress__sparkle--active"
                    style={{ position: 'absolute', top: '0', left: '0' }}
                  />
                )}
              </div>
            );
          })}
        </div>
      </div>

      {/* Labels */}
      {showLabels && (
        <div className="milestone-progress__labels">
          {milestones.map((ms, i) => {
            const reached = ms.reached ?? clamped >= ms.position;
            return (
              <span
                key={i}
                className={[
                  'milestone-progress__label',
                  reached ? 'milestone-progress__label--reached' : '',
                ]
                  .filter(Boolean)
                  .join(' ')}
                style={{ left: `${ms.position}%` }}
              >
                {ms.label}
              </span>
            );
          })}
        </div>
      )}
    </div>
  );
}

export default MilestoneProgress;
