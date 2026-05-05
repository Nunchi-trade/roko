import { useEffect, useRef, useState, useCallback } from 'react';
import './ArtifactGem.css';

interface ArtifactGemProps {
  type: 'episode' | 'insight' | 'hdc' | 'knowledge';
  significance: number;
  label?: string;
  size?: 'sm' | 'md' | 'lg';
  onClick?: () => void;
  animate?: boolean;
  className?: string;
}

/** Map 0-1 significance to visual tier. */
function toTier(s: number): 'ghost' | 'dim' | 'bright' | 'crystallized' {
  const v = Math.max(0, Math.min(1, s));
  if (v < 0.3) return 'ghost';
  if (v < 0.6) return 'dim';
  if (v < 0.8) return 'bright';
  return 'crystallized';
}

/** Number of sparkle particles based on significance. */
function sparkleCount(s: number): number {
  if (s < 0.3) return 2;
  if (s < 0.6) return 3;
  if (s < 0.8) return 4;
  return 5;
}

export default function ArtifactGem({
  type,
  significance,
  label,
  size = 'md',
  onClick,
  animate = true,
  className,
}: ArtifactGemProps) {
  const [animating, setAnimating] = useState(false);
  const [ripple, setRipple] = useState(false);
  const mountedRef = useRef(false);
  const tier = toTier(significance);
  const count = sparkleCount(significance);

  // Trigger entrance animation on mount
  useEffect(() => {
    if (!animate || mountedRef.current) return;
    mountedRef.current = true;
    setAnimating(true);
    const timeout = setTimeout(() => setAnimating(false), 600);
    return () => clearTimeout(timeout);
  }, [animate]);

  const handleClick = useCallback(() => {
    if (!onClick) return;
    setRipple(true);
    onClick();
    setTimeout(() => setRipple(false), 400);
  }, [onClick]);

  const classes = [
    'artifact-gem',
    `artifact-gem--${type}`,
    `artifact-gem--${size}`,
    animating ? 'artifact-gem--animate-enter' : '',
    onClick ? 'artifact-gem--clickable' : '',
    className ?? '',
  ]
    .filter(Boolean)
    .join(' ');

  const sparkles: number[] = [];
  for (let i = 1; i <= count; i++) sparkles.push(i);

  return (
    <span
      className={classes}
      data-tier={tier}
      onClick={handleClick}
      role={onClick ? 'button' : undefined}
      tabIndex={onClick ? 0 : undefined}
      onKeyDown={
        onClick
          ? (e) => {
              if (e.key === 'Enter' || e.key === ' ') handleClick();
            }
          : undefined
      }
    >
      <span className="artifact-gem__shape">
        <span className="artifact-gem__inner" />
      </span>

      <span className="artifact-gem__flash" />

      {sparkles.map((n) => (
        <span key={n} className={`artifact-gem__sparkle artifact-gem__sparkle--${n}`} />
      ))}

      <span className={`artifact-gem__ripple${ripple ? ' artifact-gem__ripple--active' : ''}`} />

      {label && <span className="artifact-gem__tooltip">{label}</span>}
    </span>
  );
}
