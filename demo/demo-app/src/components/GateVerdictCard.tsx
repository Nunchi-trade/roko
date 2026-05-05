import { useEffect, useRef, useState } from 'react';
import './GateVerdictCard.css';

/* ── types ── */

type GateStatus = 'pass' | 'fail' | 'pending' | 'running' | 'skip' | 'warning';

export type GateType = 'compile' | 'test' | 'clippy' | 'diff' | string;

export interface GateEntry {
  name: string;
  status: GateStatus;
  type?: GateType;
  durationMs?: number;
  score?: number;
  message?: string;
}

export interface GateVerdictCardProps {
  gates: GateEntry[];
  layout?: 'horizontal' | 'vertical';
  compact?: boolean;
  showConnectors?: boolean;
  showSummary?: boolean;
  onGateClick?: (name: string) => void;
  className?: string;
}

/* ── SVG icons with path-draw animation ── */

function CheckmarkSvg({ size = 18 }: { size?: number }) {
  return (
    <svg viewBox="0 0 18 18" width={size} height={size} aria-hidden>
      <path
        className="gate-svg-check"
        d="M3.5 9.5 L7 13 L14.5 4.5"
      />
    </svg>
  );
}

function CrossSvg({ size = 18 }: { size?: number }) {
  return (
    <svg viewBox="0 0 18 18" width={size} height={size} aria-hidden>
      <path className="gate-svg-cross" d="M4 4 L14 14" />
      <path className="gate-svg-cross gate-svg-cross--2" d="M14 4 L4 14" />
    </svg>
  );
}

function WarningSvg({ size = 18 }: { size?: number }) {
  return (
    <svg viewBox="0 0 18 18" width={size} height={size} aria-hidden>
      <path
        className="gate-svg-warning"
        d="M9 2 L17 16 L1 16 Z"
      />
      <line className="gate-svg-warning-bang" x1="9" y1="7" x2="9" y2="11" />
      <circle className="gate-svg-warning-dot" cx="9" cy="13.5" r="0.8" />
    </svg>
  );
}

/* ── Gate type icons (small, identifies the gate kind) ── */

function GearIcon({ size = 12 }: { size?: number }) {
  return (
    <svg viewBox="0 0 16 16" width={size} height={size} className="gate-type-icon" aria-hidden>
      <path
        d="M8 5.5a2.5 2.5 0 100 5 2.5 2.5 0 000-5zM6.5 1h3l.4 2.1a5.5 5.5 0 011.3.7L13.3 2.7l2.1 2.1-1.1 2.1c.3.4.5.8.7 1.3L17 8.6v3l-2.1.4a5.5 5.5 0 01-.7 1.3l1.1 2.1-2.1 2.1-2.1-1.1c-.4.3-.8.5-1.3.7L9.5 19h-3l-.4-2.1a5.5 5.5 0 01-1.3-.7L2.7 17.3.6 15.2l1.1-2.1a5.5 5.5 0 01-.7-1.3L-1 11.4v-3l2.1-.4c.2-.5.4-.9.7-1.3L.7 4.6 2.8 2.5l2.1 1.1c.4-.3.8-.5 1.3-.7z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
        transform="translate(0 -1) scale(0.88)"
      />
    </svg>
  );
}

function TestIcon({ size = 12 }: { size?: number }) {
  return (
    <svg viewBox="0 0 16 16" width={size} height={size} className="gate-type-icon" aria-hidden>
      <path
        d="M3 8l3.5 3.5L13 4"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function ClippyIcon({ size = 12 }: { size?: number }) {
  return (
    <svg viewBox="0 0 16 16" width={size} height={size} className="gate-type-icon" aria-hidden>
      <path
        d="M3 13V5h2V3h6v2h2v8H3zM6 3V1.5a2 2 0 014 0V3"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <line x1="6" y1="7" x2="10" y2="7" stroke="currentColor" strokeWidth="1" strokeLinecap="round" />
      <line x1="6" y1="9.5" x2="10" y2="9.5" stroke="currentColor" strokeWidth="1" strokeLinecap="round" />
    </svg>
  );
}

function DiffIcon({ size = 12 }: { size?: number }) {
  return (
    <svg viewBox="0 0 16 16" width={size} height={size} className="gate-type-icon" aria-hidden>
      <rect x="2" y="1" width="12" height="14" rx="1.5" fill="none" stroke="currentColor" strokeWidth="1.2" />
      <line x1="5" y1="5.5" x2="7" y2="5.5" stroke="var(--success, #7a8a78)" strokeWidth="1.3" strokeLinecap="round" />
      <line x1="6" y1="4.5" x2="6" y2="6.5" stroke="var(--success, #7a8a78)" strokeWidth="1.3" strokeLinecap="round" />
      <line x1="9" y1="10.5" x2="11" y2="10.5" stroke="var(--rose-bright, #d48a6e)" strokeWidth="1.3" strokeLinecap="round" />
    </svg>
  );
}

function GateTypeIcon({ type, size = 12 }: { type?: GateType; size?: number }) {
  switch (type) {
    case 'compile': return <GearIcon size={size} />;
    case 'test': return <TestIcon size={size} />;
    case 'clippy': return <ClippyIcon size={size} />;
    case 'diff': return <DiffIcon size={size} />;
    default: return null;
  }
}

/* ── Score mini bar ── */

function ScoreMiniBar({ score }: { score: number }) {
  const clamped = Math.max(0, Math.min(100, score));
  const barColor =
    clamped >= 80 ? 'var(--success, #7a8a78)' :
    clamped >= 50 ? 'var(--warning, #d8a878)' :
    'var(--rose-bright, #d48a6e)';

  return (
    <span className="gate-cell__score-bar">
      <span
        className="gate-cell__score-bar-fill"
        style={{ width: `${clamped}%`, background: barColor }}
      />
    </span>
  );
}

/* ── Animated score counter ── */

function AnimatedScore({ target, size = 'md' }: { target: number; size?: 'sm' | 'md' }) {
  const [displayed, setDisplayed] = useState(0);
  const rafRef = useRef<number>(0);

  useEffect(() => {
    const start = performance.now();
    const duration = 700;

    function tick(now: number) {
      const elapsed = now - start;
      const progress = Math.min(elapsed / duration, 1);
      const eased = progress === 1 ? 1 : 1 - Math.pow(2, -10 * progress);
      setDisplayed(Math.round(target * eased));
      if (progress < 1) {
        rafRef.current = requestAnimationFrame(tick);
      }
    }

    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
  }, [target]);

  return (
    <span className={`gate-cell__score gate-cell__score--${size}`}>
      {displayed}%
    </span>
  );
}

/* ── Success particles (flies outward from the cell) ── */

function SuccessParticles() {
  return (
    <span className="gate-cell__particles" aria-hidden>
      <span className="gate-cell__particle" />
      <span className="gate-cell__particle" />
      <span className="gate-cell__particle" />
      <span className="gate-cell__particle" />
      <span className="gate-cell__particle" />
    </span>
  );
}

/* braille spinner frames */
const BRAILLE_FRAMES = [
  '\u280B', '\u2819', '\u2839', '\u2838',
  '\u283C', '\u2834', '\u2826', '\u2827',
  '\u2807', '\u280F',
];

/* ── helpers ── */

function connectorStatus(left: GateStatus, right: GateStatus): string {
  if (left === 'fail' || right === 'fail') return 'fail';
  if (left === 'warning' || right === 'warning') return 'warning';
  if (left === 'running' || right === 'running') return 'running';
  if (left === 'pass' && right === 'pass') return 'pass';
  return 'pending';
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

/* ── braille spinner sub-component ── */

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

/* ── tooltip on hover ── */

function GateTooltip({ gate }: { gate: GateEntry }) {
  return (
    <span className="gate-cell__tooltip">
      <span className="gate-cell__tooltip-name">{gate.name}</span>
      <span className="gate-cell__tooltip-status">{gate.status}</span>
      {gate.message && (
        <span className="gate-cell__tooltip-message">{gate.message}</span>
      )}
      {gate.durationMs != null && (
        <span className="gate-cell__tooltip-duration">{formatDuration(gate.durationMs)}</span>
      )}
      {gate.score != null && (
        <span className="gate-cell__tooltip-score">Score: {gate.score}%</span>
      )}
    </span>
  );
}

/* ── summary row ── */

function SummaryRow({ gates }: { gates: GateEntry[] }) {
  const total = gates.length;
  const passed = gates.filter((g) => g.status === 'pass').length;
  const pct = total > 0 ? Math.round((passed / total) * 100) : 0;

  return (
    <div className="gate-summary-row">
      <span className="gate-summary-row__text">
        {passed}/{total} passed
      </span>
      <span className="gate-summary-row__bar">
        <span
          className="gate-summary-row__bar-fill"
          style={{ width: `${pct}%` }}
        />
      </span>
      <span className="gate-summary-row__pct">{pct}%</span>
    </div>
  );
}

/* ── expanded fail detail (shown in vertical non-compact) ── */

function GateFailDetail({ gate }: { gate: GateEntry }) {
  if (gate.status !== 'fail' || !gate.message) return null;
  return (
    <div className="gate-cell__fail-detail">
      <span className="gate-cell__fail-detail-label">error</span>
      <span className="gate-cell__fail-detail-msg">{gate.message}</span>
    </div>
  );
}

/* ── main component ── */

export default function GateVerdictCard({
  gates,
  layout = 'horizontal',
  compact = false,
  showConnectors = true,
  showSummary = false,
  onGateClick,
  className,
}: GateVerdictCardProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const allPass = gates.length > 0 && gates.every((g) => g.status === 'pass');
  const expanded = layout === 'vertical' && !compact;

  const containerCls = [
    'gate-verdict-card',
    `gate-verdict-card--${layout}`,
    compact && 'gate-verdict-card--compact',
    expanded && 'gate-verdict-card--expanded',
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
              {/* gate type icon (small, upper-left of cell in expanded) */}
              {gate.type && (
                <span className="gate-cell__type-icon">
                  <GateTypeIcon type={gate.type} size={compact ? 10 : 12} />
                </span>
              )}

              {/* icon / spinner / SVG */}
              <span className="gate-cell__icon">
                {gate.status === 'running' ? (
                  <BrailleSpinner />
                ) : gate.status === 'pass' ? (
                  <CheckmarkSvg size={compact ? 14 : 18} />
                ) : gate.status === 'fail' ? (
                  <CrossSvg size={compact ? 14 : 18} />
                ) : gate.status === 'warning' ? (
                  <WarningSvg size={compact ? 14 : 18} />
                ) : gate.status === 'skip' ? (
                  '\u2013'
                ) : (
                  '\u25CB'
                )}
              </span>

              {/* name */}
              <span className="gate-cell__name">{gate.name}</span>

              {/* animated score + mini bar */}
              {gate.score != null && !compact && (
                <span className="gate-cell__score-group">
                  <ScoreMiniBar score={gate.score} />
                  <AnimatedScore target={gate.score} size="md" />
                </span>
              )}

              {/* duration (hidden in compact) */}
              {gate.durationMs != null && !compact && (
                <span className="gate-cell__duration">
                  {formatDuration(gate.durationMs)}
                </span>
              )}

              {/* brief message (hidden in compact; in expanded, fail detail is below) */}
              {gate.message && !compact && !expanded && (
                <span className="gate-cell__message">{gate.message}</span>
              )}

              {/* expanded fail detail (vertical non-compact only) */}
              {expanded && <GateFailDetail gate={gate} />}

              {/* success particles on pass */}
              {gate.status === 'pass' && <SuccessParticles />}

              {/* tooltip on hover */}
              <GateTooltip gate={gate} />
            </div>
          </GateCellFragment>
        );
      })}

      {/* summary row */}
      {showSummary && gates.length > 0 && <SummaryRow gates={gates} />}
    </div>
  );
}

/** Transparent wrapper so we can return two sibling elements per gate
 *  (connector + cell) without adding a wrapper div that breaks flex layout. */
function GateCellFragment({ children }: { children: React.ReactNode }) {
  return <>{children}</>;
}
