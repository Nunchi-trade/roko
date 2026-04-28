import type { ReactNode } from 'react';

/* ── MosaicCell ─────────────────────────────────────────── */

interface MosaicCellProps {
  label: string;
  value: ReactNode;
  sub?: string;
  color?: 'rose' | 'bone' | 'dream' | 'success' | 'warning';
  mono?: boolean;
}

export function MosaicCell({ label, value, sub, color, mono }: MosaicCellProps) {
  const colorVar = color === 'bone' ? 'var(--bone-bright)'
    : color === 'dream' ? 'var(--dream-bright)'
    : color === 'success' ? 'var(--success)'
    : color === 'warning' ? 'var(--warning)'
    : 'var(--rose-glow)';

  return (
    <div className="cell">
      <div className="k">{label}</div>
      <div className={`v${mono ? ' mono' : ''}`} style={{ color: colorVar }}>
        {value}
      </div>
      {sub && <div className="sub">{sub}</div>}
    </div>
  );
}

/* ── Mosaic ──────────────────────────────────────────────── */

interface MosaicProps {
  columns: 2 | 3 | 4 | 5 | 6;
  children: ReactNode;
  className?: string;
  style?: React.CSSProperties;
}

/**
 * Grid of metric cells with 1px gap (gap IS the border color).
 * Cells have bg-void background. Uses canonical .mosaic from rosedust.css.
 */
export default function Mosaic({ columns, children, className, style }: MosaicProps) {
  return (
    <div
      className={`mosaic${className ? ` ${className}` : ''}`}
      style={{
        gridTemplateColumns: `repeat(${columns}, 1fr)`,
        ...style,
      }}
    >
      {children}
    </div>
  );
}
