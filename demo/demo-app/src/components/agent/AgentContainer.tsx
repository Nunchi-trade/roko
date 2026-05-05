import type { ReactNode } from 'react';
import './AgentContainer.css';
import AgentHeartbeat from './AgentHeartbeat';
import { agentColor } from './utils';

type ContainerStatus = 'idle' | 'active' | 'busy' | 'error' | 'offline';

interface AgentContainerProps {
  name: string;
  role?: string;
  status?: ContainerStatus;
  color?: string;
  heartbeat?: 'fast' | 'medium' | 'slow' | 'none';
  children: ReactNode;
  className?: string;
}

const STATUS_HEARTBEAT_MAP: Record<ContainerStatus, 'healthy' | 'degraded' | 'critical' | 'offline'> = {
  idle: 'healthy',
  active: 'healthy',
  busy: 'degraded',
  error: 'critical',
  offline: 'offline',
};

const DEFAULT_HEARTBEAT: Record<ContainerStatus, 'fast' | 'medium' | 'slow' | 'none'> = {
  idle: 'slow',
  active: 'fast',
  busy: 'medium',
  error: 'fast',
  offline: 'none',
};

export default function AgentContainer({
  name,
  role,
  status = 'idle',
  color,
  heartbeat,
  children,
  className,
}: AgentContainerProps) {
  const accent = color ?? agentColor(name);
  const speed = heartbeat ?? DEFAULT_HEARTBEAT[status];
  const hbStatus = STATUS_HEARTBEAT_MAP[status];

  return (
    <div
      className={[
        'agent-container',
        `agent-container--${status}`,
        className,
      ].filter(Boolean).join(' ')}
      style={{
        borderLeftColor: status === 'error' ? undefined : accent,
      }}
    >
      <div className="agent-container__header">
        <AgentHeartbeat speed={speed} status={hbStatus} size={8} />
        <span className="agent-container__name">{name}</span>
        {role && <span className="agent-container__role">{role}</span>}
        <span className="agent-container__status">{status}</span>
      </div>
      <div className="agent-container__body">
        {children}
      </div>
    </div>
  );
}
