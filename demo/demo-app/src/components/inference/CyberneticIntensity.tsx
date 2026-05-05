import type { ReactNode } from 'react';
import './CyberneticIntensity.css';

interface CyberneticIntensityProps {
  value: number;
  children: ReactNode;
  showLabel?: boolean;
  variant?: 'background' | 'border' | 'glow' | 'all';
  threshold?: number;
  className?: string;
}

/** Map a 0-1 float to the 0-10 integer intensity scale. */
function toIntensity(v: number): number {
  const clamped = Math.max(0, Math.min(1, v));
  return Math.round(clamped * 10);
}

export default function CyberneticIntensity({
  value,
  children,
  showLabel = false,
  variant = 'all',
  threshold: _threshold = 0.8,
  className,
}: CyberneticIntensityProps) {
  const intensity = toIntensity(value);
  const pct = Math.round(Math.max(0, Math.min(1, value)) * 100);

  const variantClass =
    variant === 'background'
      ? ' cybernetic-intensity--background-only'
      : variant === 'border'
        ? ' cybernetic-intensity--border-only'
        : variant === 'glow'
          ? ' cybernetic-intensity--glow-only'
          : '';

  return (
    <div
      className={`cybernetic-intensity${variantClass}${className ? ` ${className}` : ''}`}
      data-intensity={intensity}
    >
      {showLabel && (
        <span className="cybernetic-intensity__label">{pct}%</span>
      )}
      {children}
    </div>
  );
}
