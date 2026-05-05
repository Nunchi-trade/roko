import './AgentHeartbeat.css';

interface AgentHeartbeatProps {
  speed: 'fast' | 'medium' | 'slow' | 'none';
  status?: 'healthy' | 'degraded' | 'critical' | 'offline';
  size?: number;
  showLabel?: boolean;
  className?: string;
}

const LABEL_MAP: Record<string, [string, string]> = {
  'fast-healthy':    ['\u25C9', 'ACTIVE'],
  'fast-degraded':   ['\u25C9', 'ACTIVE'],
  'fast-critical':   ['\u25C9', 'ACTIVE'],
  'medium-healthy':  ['\u25C9', 'IDLE'],
  'medium-degraded': ['\u25C9', 'IDLE'],
  'medium-critical': ['\u25C9', 'DEGRADED'],
  'slow-healthy':    ['\u25CB', 'STANDBY'],
  'slow-degraded':   ['\u25CB', 'STANDBY'],
  'slow-critical':   ['\u25CB', 'CRITICAL'],
  'none-healthy':    ['\u25CB', 'IDLE'],
  'none-degraded':   ['\u25CB', 'DEGRADED'],
  'none-critical':   ['\u25CB', 'CRITICAL'],
  'none-offline':    ['\u25CB', 'OFFLINE'],
  'fast-offline':    ['\u25CB', 'OFFLINE'],
  'medium-offline':  ['\u25CB', 'OFFLINE'],
  'slow-offline':    ['\u25CB', 'OFFLINE'],
};

/** Clamp to sizes we have CSS classes for */
function clampSize(px: number): number {
  const stops = [8, 10, 12, 14, 16];
  let best = stops[0];
  for (const s of stops) {
    if (Math.abs(s - px) < Math.abs(best - px)) best = s;
  }
  return best;
}

export default function AgentHeartbeat({
  speed,
  status = 'healthy',
  size = 8,
  showLabel = false,
  className,
}: AgentHeartbeatProps) {
  const clamped = clampSize(size);
  const key = `${speed}-${status}` as keyof typeof LABEL_MAP;
  const [icon, text] = LABEL_MAP[key] ?? ['\u25CB', 'UNKNOWN'];

  return (
    <span
      className={[
        'agent-heartbeat',
        `agent-heartbeat--${status}`,
        `agent-heartbeat--${speed}`,
        className,
      ].filter(Boolean).join(' ')}
    >
      <span className={`agent-heartbeat__dot agent-heartbeat__dot--${clamped}`} />
      {showLabel && (
        <span className="agent-heartbeat__label">
          {icon} {text}
        </span>
      )}
    </span>
  );
}
