import './Pulse.css';

interface PulseProps {
  color?: string;
  size?: number;
  animate?: boolean;
}

export function Pulse({
  color = 'var(--status-active)',
  size = 8,
  animate = true,
}: PulseProps) {
  return (
    <span
      className={`pulse${animate ? ' pulse--animate' : ''}`}
      style={{
        width: size,
        height: size,
        background: color,
        boxShadow: `0 0 ${size}px ${color}`,
      }}
    />
  );
}
