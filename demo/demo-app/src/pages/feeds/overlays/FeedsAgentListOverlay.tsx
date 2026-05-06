import { useMemo } from 'react';
import { useDataHub } from '../../../app/DataHub';

interface Props { onSelect: (agentId: string) => void; }

export default function FeedsAgentListOverlay({ onSelect }: Props) {
  const relayAgents = useDataHub((s) => s.relayAgents);
  const sorted = useMemo(
    () => [...relayAgents].sort((a, b) => (b.online ? 1 : 0) - (a.online ? 1 : 0) || a.name.localeCompare(b.name)),
    [relayAgents],
  );

  if (!sorted.length) return (
    <div style={{ minWidth: 200 }}>
      <div className="gp-label">AGENTS</div>
      <div className="gp-sub">No agents connected</div>
    </div>
  );

  return (
    <div style={{ minWidth: 220, maxHeight: 300, overflowY: 'auto' }}>
      <div className="gp-label">AGENTS ({sorted.length})</div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 2, marginTop: 4 }}>
        {sorted.map((agent) => (
          <button key={agent.agentId} onClick={() => onSelect(agent.agentId)}
            style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '3px 4px',
              borderRadius: 'var(--radius-sm)', border: 'none', background: 'transparent',
              cursor: 'pointer', width: '100%', textAlign: 'left' }}
            onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.background = 'var(--glass-bg)'; }}
            onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.background = 'transparent'; }}>
            <span className={`gp-dot ${agent.online ? 'gp-dot--live' : 'gp-dot--offline'}`} />
            <span style={{ fontFamily: 'var(--mono)', fontSize: 11, fontWeight: 600,
              color: 'var(--text-primary)', flex: 1, minWidth: 0,
              overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {agent.name}
            </span>
            <span style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--text-ghost)',
              padding: '1px 4px', borderRadius: 'var(--radius-sm)',
              background: 'var(--glass-bg)', border: '1px solid var(--glass-border)' }}>
              {agent.feedCount}
            </span>
            {agent.capabilities?.slice(0, 2).map((cap) => (
              <span key={cap} style={{ fontFamily: 'var(--mono)', fontSize: 8,
                padding: '0 4px', borderRadius: 'var(--radius-sm)',
                background: 'color-mix(in srgb, var(--status-active) 15%, transparent)',
                color: 'var(--status-active)', letterSpacing: '0.04em', textTransform: 'uppercase' }}>
                {cap}
              </span>
            ))}
          </button>
        ))}
      </div>
    </div>
  );
}
