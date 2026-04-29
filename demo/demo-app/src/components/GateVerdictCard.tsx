import { useEffect, useRef, useState } from 'react';
import './GateVerdictCard.css';

/* ── types ── */

type GateStatus = 'pass' | 'fail' | 'pending' | 'running' | 'skip';

export interface GateEntry {
  name: string;
  status: GateStatus;
  durationMs?: number;
  message?: string;
}

export interface GateVerdictCardProps {
  gates: GateEntry[];
  layout?: 'horizontal' | 'vertical';
  compact?: boolean;
  showConnectors?: boolean;
  onGateClick?: (name: string) => void;
  className?: string;
}

/* ── icons per status ── */

const STATUS_ICON: Record<GateStatus, string> = {
  pass: '\u2308\u2713\u230B',    // "ceil-check-floor"
  fail: '\u2308\u2717\u230B',    // "ceil-cross-floor"
  pending: '\u25CB',              // empty circle
  running: '',                    // handled by spinner
  skip: '\u2013',                 // en-dash
};

/* braille spinner frames */
const BRAILLE_FRAMES = [
  '\u280B', '\u2819', '\u2839', '\u2838',
  '\u283C', '\u2834', '\u2826', '\u2827',
  '\u2807', '\u280F',
];

/* ── helpers ── */

/** Determine connector style based on the gate to its left and to its right. */
function connectorStatus(left: GateStatus, right: GateStatus): string {
  if (left === 'fail' || right === 'fail') return 'fail';
  if (left === 'running' || right === 'running') return 'running';
  if (left === 'pass' && right === 'pass') return 'pass';
  return 'pending';
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

/* ── braille spinner sub-component (pure CSS would need content steps;
     we use a tiny hook to cycle through frames instead) ── */

function BrailleSpinner() {
  const [frame, setFrame] = useState(0);

  useEffect(() => {
    const id = setInterval(() => {
      setFrame((f) => (f + 1) % BRAILLE_FRAMES.length);
    }, 80);
    return () => clearInterval(id);
  }, []);

  return (
    <span className="gate-cell__spinner" aria-label="running">
      {BRAILLE_FRAMES[frame]}
    </span>
  );
}

/* ── main component ── */

export default function GateVerdictCard({
  gates,
  layout = 'horizontal',
  compact = false,
  showConnectors = true,
  onGateClick,
  className,
}: GateVerdictCardProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const allPass = gates.length > 0 && gates.every((g) => g.status === 'pass');

  const containerCls = [
    'gate-verdict-card',
    `gate-verdict-card--${layout}`,
    compact && 'gate-verdict-card--compact',
    allPass && 'gate-verdict-all-pass',
    className,
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <div className={containerCls} ref={containerRef}>
      {gates.map((gate, i) => {
        const isClickable = !!onGateClick;
        const cellCls = [
          'gate-cell',
          `gate-cell--${gate.status}`,
          isClickable && 'gate-cell--clickable',
        ]
          .filter(Boolean)
          .join(' ');

        return (
          <GateCellFragment key={gate.name}>
            {/* connector before this gate (except the first) */}
            {showConnectors && i > 0 && (
              <div
                className={`gate-connector gate-connector--${connectorStatus(
                  gates[i - 1].status,
                  gate.status,
                )}`}
              />
            )}

            <div
              className={cellCls}
              title={gate.message ?? `${gate.name}: ${gate.status}`}
              onClick={isClickable ? () => onGateClick!(gate.name) : undefined}
              role={isClickable ? 'button' : undefined}
              tabIndex={isClickable ? 0 : undefined}
              onKeyDown={
                isClickable
                  ? (e) => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault();
                        onGateClick!(gate.name);
                      }
                    }
                  : undefined
              }
            >
              {/* icon / spinner */}
              <span className="gate-cell__icon">
                {gate.status === 'running' ? (
                  <BrailleSpinner />
                ) : (
                  STATUS_ICON[gate.status]
                )}
              </span>

              {/* name */}
              <span className="gate-cell__name">{gate.name}</span>

              {/* duration (hidden in compact) */}
              {gate.durationMs != null && !compact && (
                <span className="gate-cell__duration">
                  {formatDuration(gate.durationMs)}
                </span>
              )}

              {/* message (hidden in compact) */}
              {gate.message && !compact && (
                <span className="gate-cell__message">{gate.message}</span>
              )}
            </div>
          </GateCellFragment>
        );
      })}
    </div>
  );
}

/** Transparent wrapper so we can return two sibling elements per gate
 *  (connector + cell) without adding a wrapper div that breaks flex layout. */
function GateCellFragment({ children }: { children: React.ReactNode }) {
  return <>{children}</>;
}
