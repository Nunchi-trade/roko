import { useRef, useEffect } from 'react';
import './Timeline.css';

/* ── Status types ─────────────────────────────────────────── */

type StepStatus = 'done' | 'active' | 'pending' | 'failed';

interface Step {
  label: string;
  status: StepStatus;
  detail?: string;
  /** Elapsed ms — displayed as "Xs" when done */
  durationMs?: number;
}

interface TimelineProps {
  steps: Step[];
  /** Reduces spacing for tight sidebar layouts */
  compact?: boolean;
}

/* ── SVG icons ────────────────────────────────────────────── */

function CheckIcon() {
  return (
    <svg className="tl-icon tl-icon-check" viewBox="0 0 16 16" width="10" height="10">
      <path
        d="M3 8.5 L6.5 12 L13 4"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function CrossIcon() {
  return (
    <svg className="tl-icon tl-icon-cross" viewBox="0 0 16 16" width="10" height="10">
      <path
        d="M4 4 L12 12 M12 4 L4 12"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.2"
        strokeLinecap="round"
      />
    </svg>
  );
}

function SpinnerIcon() {
  return (
    <svg className="tl-icon tl-icon-spinner" viewBox="0 0 16 16" width="10" height="10">
      <circle
        cx="8" cy="8" r="5.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeDasharray="20 14"
      />
    </svg>
  );
}

/* ── Helpers ──────────────────────────────────────────────── */

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

/* ── Component ────────────────────────────────────────────── */

export default function Timeline({ steps, compact }: TimelineProps) {
  const prevLenRef = useRef(0);

  // Track when new steps appear so we can animate them in
  useEffect(() => {
    prevLenRef.current = steps.length;
  }, [steps.length]);

  const doneCount = steps.filter((s) => s.status === 'done').length;
  const total = steps.length;

  return (
    <div className={`timeline ${compact ? 'timeline--compact' : ''}`}>
      {/* ── Connecting line (background + fill) ── */}
      {steps.length > 1 && <TimelineTrack steps={steps} compact={compact} />}

      {/* ── Steps ── */}
      {steps.map((step, i) => {
        const isNew = i >= prevLenRef.current;
        return (
          <div
            key={i}
            className={`timeline-step timeline-${step.status}`}
            style={{
              animationDelay: isNew ? `${(i - prevLenRef.current) * 80}ms` : '0ms',
            }}
            data-new={isNew || undefined}
          >
            <div className="timeline-marker">
              {step.status === 'done' && <CheckIcon />}
              {step.status === 'active' && <SpinnerIcon />}
              {step.status === 'failed' && <CrossIcon />}
              {/* pending: empty circle, no icon */}
            </div>
            <div className="timeline-content">
              <div className="timeline-label">{step.label}</div>
              {(step.detail || (step.status === 'done' && step.durationMs != null)) && (
                <div className="timeline-detail">
                  {step.detail}
                  {step.status === 'done' && step.durationMs != null && (
                    <span className="timeline-duration">
                      {step.detail ? ' · ' : ''}
                      {formatDuration(step.durationMs)}
                    </span>
                  )}
                </div>
              )}
            </div>
          </div>
        );
      })}

      {/* ── Progress fraction ── */}
      {total > 0 && (
        <div className="timeline-progress">
          {doneCount}/{total}
        </div>
      )}
    </div>
  );
}

/* ── Animated track (vertical line) ───────────────────────── */

function TimelineTrack({ steps, compact }: { steps: Step[]; compact?: boolean }) {
  const total = steps.length;
  if (total < 2) return null;

  // Calculate fill ratio: done steps fill their segments, active fills halfway
  let filled = 0;
  for (const step of steps) {
    if (step.status === 'done') filled += 1;
    else if (step.status === 'active') {
      filled += 0.5;
      break;
    } else break;
  }
  const pct = Math.min((filled / (total - 1)) * 100, 100);
  const stepGap = compact ? 28 : 36; // must match CSS gap

  return (
    <div
      className="timeline-track"
      style={{
        // Track spans from first marker center to last marker center
        top: `${compact ? 13 : 15}px`,
        height: `${(total - 1) * stepGap}px`,
      }}
    >
      <div className="timeline-track-bg" />
      <div
        className="timeline-track-fill"
        style={{ height: `${pct}%` }}
      />
    </div>
  );
}
