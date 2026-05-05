import type { CSSProperties } from 'react';
import './AnimatedIcons.css';

/* ── Shared props ──────────────────────────────────────────── */

export interface AnimatedIconProps {
  size?: number;
  color?: string;
  className?: string;
  delay?: number;
}

function iconStyle(size: number, color?: string, delay?: number): CSSProperties {
  const s: CSSProperties = {
    width: size,
    height: size,
    color: color ?? 'currentColor',
  };
  if (delay) s.animationDelay = `${delay}ms`;
  return s;
}

/* 1 ── CheckmarkIcon ───────────────────────────────────────── */
/*  Circle draws in, then checkmark strokes in.                 */

export function CheckmarkIcon({ size = 20, color, className, delay }: AnimatedIconProps) {
  return (
    <span className={`aicon aicon-checkmark${className ? ` ${className}` : ''}`} style={iconStyle(size, color, delay)}>
      <svg viewBox="0 0 24 24">
        <circle className="aicon-circle" cx="12" cy="12" r="10" fill="none" />
        <path className="aicon-check" d="M7 12.5l3 3 7-7" fill="none" />
      </svg>
    </span>
  );
}

/* 2 ── CrossIcon ───────────────────────────────────────────── */
/*  Circle draws in, X strokes in with shake.                   */

export function CrossIcon({ size = 20, color, className, delay }: AnimatedIconProps) {
  return (
    <span className={`aicon aicon-cross${className ? ` ${className}` : ''}`} style={iconStyle(size, color, delay)}>
      <svg viewBox="0 0 24 24">
        <circle className="aicon-circle" cx="12" cy="12" r="10" fill="none" />
        <path className="aicon-x1" d="M8 8l8 8" fill="none" />
        <path className="aicon-x2" d="M16 8l-8 8" fill="none" />
      </svg>
    </span>
  );
}

/* 3 ── SpinnerIcon ─────────────────────────────────────────── */
/*  Multiple arcs spinning at different speeds.                 */

export function SpinnerIcon({ size = 20, color, className }: AnimatedIconProps) {
  return (
    <span className={`aicon aicon-spinner${className ? ` ${className}` : ''}`} style={iconStyle(size, color)}>
      <svg viewBox="0 0 24 24">
        <circle className="aicon-arc1" cx="12" cy="12" r="10" fill="none" />
        <circle className="aicon-arc2" cx="12" cy="12" r="7" fill="none" />
        <circle className="aicon-arc3" cx="12" cy="12" r="4" fill="none" />
      </svg>
    </span>
  );
}

/* 4 ── ArrowIcon ───────────────────────────────────────────── */
/*  Arrow that animates pointing direction changes smoothly.    */

export interface ArrowIconProps extends AnimatedIconProps {
  direction?: 'up' | 'down' | 'left' | 'right';
}

const ARROW_ROTATIONS = { up: -90, right: 0, down: 90, left: 180 };

export function ArrowIcon({ size = 20, color, className, direction = 'right' }: ArrowIconProps) {
  return (
    <span className={`aicon aicon-arrow${className ? ` ${className}` : ''}`} style={iconStyle(size, color)}>
      <svg viewBox="0 0 24 24" style={{ transform: `rotate(${ARROW_ROTATIONS[direction]}deg)` }}>
        <path d="M5 12h14" fill="none" />
        <path d="M13 6l6 6-6 6" fill="none" />
      </svg>
    </span>
  );
}

/* 5 ── PulseIcon ───────────────────────────────────────────── */
/*  Dot with expanding/fading ring for "live" indicators.       */

export function PulseIcon({ size = 20, color, className }: AnimatedIconProps) {
  return (
    <span className={`aicon aicon-pulse${className ? ` ${className}` : ''}`} style={iconStyle(size, color)}>
      <svg viewBox="0 0 24 24">
        <circle className="aicon-ring" cx="12" cy="12" r="4" />
        <circle className="aicon-dot" cx="12" cy="12" r="3" />
      </svg>
    </span>
  );
}

/* 6 ── ChevronIcon ─────────────────────────────────────────── */
/*  Chevron that animates rotation for expand/collapse.         */

export interface ChevronIconProps extends AnimatedIconProps {
  expanded?: boolean;
}

export function ChevronIcon({ size = 20, color, className, expanded }: ChevronIconProps) {
  return (
    <span className={`aicon aicon-chevron${expanded ? ' aicon-expanded' : ''}${className ? ` ${className}` : ''}`} style={iconStyle(size, color)}>
      <svg viewBox="0 0 24 24">
        <path d="M6 9l6 6 6-6" fill="none" />
      </svg>
    </span>
  );
}

/* 7 ── WarningIcon ─────────────────────────────────────────── */
/*  Triangle draws in, exclamation bounces.                     */

export function WarningIcon({ size = 20, color, className, delay }: AnimatedIconProps) {
  return (
    <span className={`aicon aicon-warning${className ? ` ${className}` : ''}`} style={iconStyle(size, color, delay)}>
      <svg viewBox="0 0 24 24">
        <path className="aicon-tri" d="M12 3L2 21h20Z" fill="none" />
        <g className="aicon-bang">
          <line x1="12" y1="9" x2="12" y2="14" stroke="currentColor" strokeWidth="2" />
          <circle cx="12" cy="17" r="1" fill="currentColor" stroke="none" />
        </g>
      </svg>
    </span>
  );
}

/* 8 ── InfoIcon ────────────────────────────────────────────── */
/*  Circle draws in, "i" fades in.                              */

export function InfoIcon({ size = 20, color, className, delay }: AnimatedIconProps) {
  return (
    <span className={`aicon aicon-info${className ? ` ${className}` : ''}`} style={iconStyle(size, color, delay)}>
      <svg viewBox="0 0 24 24">
        <circle className="aicon-circle" cx="12" cy="12" r="10" fill="none" />
        <g className="aicon-i">
          <circle cx="12" cy="8" r="1" fill="currentColor" stroke="none" />
          <line x1="12" y1="11" x2="12" y2="17" stroke="currentColor" strokeWidth="2" />
        </g>
      </svg>
    </span>
  );
}

/* 9 ── StarIcon ────────────────────────────────────────────── */
/*  Star that fills with animation for ratings/favorites.       */

export interface StarIconProps extends AnimatedIconProps {
  filled?: boolean;
}

export function StarIcon({ size = 20, color, className, filled }: StarIconProps) {
  return (
    <span className={`aicon aicon-star${filled ? ' aicon-filled' : ''}${className ? ` ${className}` : ''}`} style={iconStyle(size, color)}>
      <svg viewBox="0 0 24 24">
        <path
          className="aicon-star-path"
          d="M12 2l3.09 6.26L22 9.27l-5 4.87L18.18 22 12 18.56 5.82 22 7 14.14l-5-4.87 6.91-1.01Z"
        />
      </svg>
    </span>
  );
}

/* 10 ── GearIcon ───────────────────────────────────────────── */
/*  Gear that rotates smoothly on hover.                        */

export function GearIcon({ size = 20, color, className }: AnimatedIconProps) {
  return (
    <span className={`aicon aicon-gear${className ? ` ${className}` : ''}`} style={iconStyle(size, color)}>
      <svg viewBox="0 0 24 24">
        <circle cx="12" cy="12" r="3" fill="none" />
        <path
          d="M12 2.7V1m0 22v-1.7M22 12h1.3M.7 12H2m17.1-7.1.8-.8M4.1 19.9l.8-.8m14.2 0 .8.8M4.1 4.1l.8.8"
          fill="none"
        />
        <circle cx="12" cy="12" r="8" fill="none" strokeDasharray="3 3" />
      </svg>
    </span>
  );
}

/* 11 ── BoltIcon ───────────────────────────────────────────── */
/*  Lightning bolt with brief flash effect.                     */

export function BoltIcon({ size = 20, color, className, delay }: AnimatedIconProps) {
  return (
    <span className={`aicon aicon-bolt${className ? ` ${className}` : ''}`} style={iconStyle(size, color, delay)}>
      <svg viewBox="0 0 24 24">
        <path className="aicon-bolt-path" d="M13 2L4 14h7l-1 8 9-12h-7l1-8Z" fill="none" />
        <circle className="aicon-flash" cx="12" cy="12" r="6" opacity="0" />
      </svg>
    </span>
  );
}

/* 12 ── WaveformIcon ───────────────────────────────────────── */
/*  Animated sound-wave bars for "processing" state.            */

export function WaveformIcon({ size = 20, color, className }: AnimatedIconProps) {
  const barWidth = 2;
  const gap = 2.5;
  const bars = 5;
  const totalWidth = bars * barWidth + (bars - 1) * gap;
  const startX = (24 - totalWidth) / 2;

  return (
    <span className={`aicon aicon-waveform${className ? ` ${className}` : ''}`} style={iconStyle(size, color)}>
      <svg viewBox="0 0 24 24">
        {Array.from({ length: bars }, (_, i) => (
          <rect
            key={i}
            className="aicon-bar"
            x={startX + i * (barWidth + gap)}
            y={5}
            width={barWidth}
            height={14}
            rx={1}
            fill="currentColor"
            stroke="none"
          />
        ))}
      </svg>
    </span>
  );
}
