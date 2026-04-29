import type { AgentIdentity } from './AgentIdentity';
import { ROLE_PALETTES } from './AgentIdentity';
import SpectreAvatar from './SpectreAvatar';
import './AgentCard.css';

export type AgentStatus = 'active' | 'idle' | 'error' | 'completed';

export interface AgentCardProps {
  identity: AgentIdentity;
  variant: 'compact' | 'standard' | 'detailed' | 'hero';
  status?: AgentStatus;
  stats?: { tokens?: number; cost?: number; duration?: number };
  onClick?: () => void;
}

/* ── Status pill ─────────────────────────────────────────── */

function StatusPill({ status }: { status: AgentStatus }) {
  return (
    <span className={`ac-status-pill ac-status-pill--${status}`}>
      {status}
    </span>
  );
}

/* ── Stat formatters ─────────────────────────────────────── */

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function fmtDuration(seconds: number): string {
  if (seconds < 60) return `${Math.round(seconds)}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${Math.round(seconds % 60)}s`;
  return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
}

/* ── AgentCard ───────────────────────────────────────────── */

export default function AgentCard({ identity, variant, status, stats, onClick }: AgentCardProps) {
  const palette = ROLE_PALETTES[identity.role];
  const roleColor = palette[0];
  const avatarSize = variant === 'compact' ? 24 : variant === 'standard' ? 40 : variant === 'detailed' ? 56 : 80;

  return (
    <div
      className={`ac-card ac-card--${variant}`}
      style={{ '--ac-role-color': roleColor } as React.CSSProperties}
      onClick={onClick}
      role={onClick ? 'button' : undefined}
      tabIndex={onClick ? 0 : undefined}
      onKeyDown={onClick ? (e) => { if (e.key === 'Enter' || e.key === ' ') onClick(); } : undefined}
    >
      <SpectreAvatar identity={identity} size={avatarSize} />

      <div className="ac-info">
        <span className="ac-name">{identity.name}</span>

        {variant !== 'compact' && (
          <span className="ac-role">{identity.role}</span>
        )}

        {variant !== 'compact' && status && (
          <StatusPill status={status} />
        )}

        {(variant === 'detailed' || variant === 'hero') && stats && (
          <div className="ac-stats">
            {stats.tokens != null && (
              <span className="ac-stat">
                <span className="ac-stat-label">tok</span>
                <span className="ac-stat-value">{fmtTokens(stats.tokens)}</span>
              </span>
            )}
            {stats.cost != null && (
              <span className="ac-stat">
                <span className="ac-stat-label">cost</span>
                <span className="ac-stat-value">${stats.cost.toFixed(2)}</span>
              </span>
            )}
            {stats.duration != null && (
              <span className="ac-stat">
                <span className="ac-stat-label">time</span>
                <span className="ac-stat-value">{fmtDuration(stats.duration)}</span>
              </span>
            )}
          </div>
        )}

        {variant === 'hero' && (
          <div className="ac-sparkline-placeholder" />
        )}
      </div>
    </div>
  );
}
