import type { ReactNode } from 'react';
import './StepProgress.css';

interface Step {
  id: string;
  label: string;
  icon?: ReactNode;
  status: 'pending' | 'active' | 'complete' | 'failed' | 'skipped';
  detail?: string;
}

interface StepProgressProps {
  steps: Step[];
  orientation?: 'horizontal' | 'vertical';
  size?: 'sm' | 'md';
  className?: string;
}

const STATUS_SYMBOLS: Record<Step['status'], string> = {
  pending: '\u25CB',   // circle
  active: '\u25CF',    // filled circle
  complete: '\u2713',  // check
  failed: '\u2715',    // x
  skipped: '\u2014',   // em dash
};

const STATUS_COLORS: Record<Step['status'], string> = {
  pending: 'var(--text-ghost)',
  active: 'var(--rose-bright)',
  complete: 'var(--status-success)',
  failed: 'var(--status-error)',
  skipped: 'var(--text-ghost)',
};

function connectorGradient(
  fromStatus: Step['status'],
  toStatus: Step['status'],
  orientation: 'horizontal' | 'vertical',
): string {
  const from = STATUS_COLORS[fromStatus];
  const to = STATUS_COLORS[toStatus];
  const dir = orientation === 'horizontal' ? 'to right' : 'to bottom';
  return `linear-gradient(${dir}, ${from}, ${to})`;
}

/** Whether the connector between two steps should play the draw animation */
function shouldDraw(from: Step['status'], to: Step['status']): boolean {
  return from === 'complete' && to === 'active';
}

export function StepProgress({
  steps,
  orientation = 'horizontal',
  size = 'md',
  className,
}: StepProgressProps) {
  return (
    <div
      className={[
        'step-progress',
        `step-progress--${orientation}`,
        `step-progress--${size}`,
        className,
      ]
        .filter(Boolean)
        .join(' ')}
    >
      {steps.map((step, i) => {
        const isLast = i === steps.length - 1;
        const next = steps[i + 1];

        return (
          <div
            key={step.id}
            className={`step-progress__step step-progress__step--${step.status}`}
          >
            {/* Circle */}
            <div className={`step-progress__circle step-progress__circle--${step.status}`}>
              {step.icon ? (
                <span className="step-progress__icon">{step.icon}</span>
              ) : (
                STATUS_SYMBOLS[step.status]
              )}
            </div>

            {/* Text content */}
            <div className="step-progress__content">
              <div className="step-progress__label">{step.label}</div>
              {step.detail && (
                <div className="step-progress__detail">{step.detail}</div>
              )}
            </div>

            {/* Connector line */}
            {!isLast && next && (
              <div className="step-progress__connector">
                <div
                  className={[
                    'step-progress__connector-fill',
                    shouldDraw(step.status, next.status)
                      ? 'step-progress__connector-fill--drawing'
                      : step.status === 'pending' && next.status === 'pending'
                        ? 'step-progress__connector-fill--pending'
                        : '',
                  ]
                    .filter(Boolean)
                    .join(' ')}
                  style={{
                    background: connectorGradient(step.status, next.status, orientation),
                  }}
                />
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

export default StepProgress;
