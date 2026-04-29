import './AgentAvatar.css';
import { agentColor } from './utils';

interface AgentAvatarProps {
  name: string;
  role?: string;
  size?: 'xs' | 'sm' | 'md';
  showTooltip?: boolean;
  className?: string;
}

export default function AgentAvatar({
  name,
  role,
  size = 'sm',
  showTooltip = true,
  className,
}: AgentAvatarProps) {
  const color = agentColor(name);
  const initials = name.slice(0, 2).toUpperCase();

  return (
    <span
      className={[
        'agent-avatar',
        `agent-avatar--${size}`,
        className,
      ].filter(Boolean).join(' ')}
      style={{
        background: color,
        boxShadow: `0 0 8px ${color}44`,
      }}
    >
      {initials}
      {showTooltip && (
        <span className="agent-avatar__tooltip">
          {name}
          {role && (
            <span className="agent-avatar__tooltip-role">
              {'\u00B7'} {role}
            </span>
          )}
        </span>
      )}
    </span>
  );
}
