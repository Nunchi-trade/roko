import { useRef, useEffect, useState, type ReactNode } from 'react';
import './CircularProgress.css';

interface CircularProgressProps {
  value: number;
  size?: number;
  strokeWidth?: number;
  label?: string;
  children?: ReactNode;
  color?: string;
  trackColor?: string;
  className?: string;
}

export function CircularProgress({
  value,
  size = 80,
  strokeWidth = 4,
  label,
  children,
  color = 'var(--rose-bright)',
  trackColor = 'var(--border-soft)',
  className,
}: CircularProgressProps) {
  const gradientId = useRef(`cp-grad-${Math.random().toString(36).slice(2, 8)}`);
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    // Trigger mount animation on next frame
    const raf = requestAnimationFrame(() => setMounted(true));
    return () => cancelAnimationFrame(raf);
  }, []);

  const clamped = Math.max(0, Math.min(100, value));
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const offset = circumference - (clamped / 100) * circumference;

  // Font size scales with ring size
  const valueFontSize = Math.max(12, Math.round(size * 0.28));

  return (
    <div
      className={['circular-progress', className].filter(Boolean).join(' ')}
      style={{ width: size, height: size }}
    >
      <svg
        className="circular-progress__svg"
        width={size}
        height={size}
        viewBox={`0 0 ${size} ${size}`}
      >
        <defs>
          <linearGradient
            id={gradientId.current}
            x1="0%"
            y1="0%"
            x2="100%"
            y2="0%"
          >
            <stop offset="0%" stopColor="var(--rose-dim)" />
            <stop offset="100%" stopColor={color} />
          </linearGradient>
        </defs>

        {/* Track */}
        <circle
          className="circular-progress__track"
          cx={size / 2}
          cy={size / 2}
          r={radius}
          stroke={trackColor}
          strokeWidth={strokeWidth}
        />

        {/* Fill */}
        <circle
          className={[
            'circular-progress__fill',
            !mounted ? 'circular-progress__fill--animate' : '',
          ]
            .filter(Boolean)
            .join(' ')}
          cx={size / 2}
          cy={size / 2}
          r={radius}
          stroke={`url(#${gradientId.current})`}
          strokeWidth={strokeWidth}
          strokeDasharray={circumference}
          strokeDashoffset={mounted ? offset : circumference}
          style={
            {
              '--cp-circumference': circumference,
              '--cp-offset': offset,
            } as React.CSSProperties
          }
        />
      </svg>

      {/* Center content */}
      <div className="circular-progress__center">
        {children ?? (
          <>
            <span
              className="circular-progress__value"
              style={{ fontSize: valueFontSize }}
            >
              {Math.round(clamped)}
            </span>
            {label && <span className="circular-progress__label">{label}</span>}
          </>
        )}
      </div>
    </div>
  );
}

export default CircularProgress;
