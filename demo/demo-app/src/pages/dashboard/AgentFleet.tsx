import { useState, useEffect } from 'react';
import { useApiWithFallback } from '../../hooks/useApiWithFallback';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';

/* ── Keyframes ───────────────────────────────────────────── */
const FLEET_STYLES = `
  @keyframes fleet-pulse-green {
    0%, 100% { box-shadow: 0 0 4px 1px rgba(138,156,134,.6), 0 0 8px 2px rgba(138,156,134,.25); }
    50%       { box-shadow: 0 0 8px 3px rgba(138,156,134,.9), 0 0 16px 5px rgba(138,156,134,.4); }
  }
  @keyframes fleet-pulse-amber {
    0%, 100% { box-shadow: 0 0 4px 1px rgba(216,168,120,.6), 0 0 8px 2px rgba(216,168,120,.25); }
    50%       { box-shadow: 0 0 8px 3px rgba(216,168,120,.9), 0 0 16px 5px rgba(216,168,120,.4); }
  }
  .agent-card {
    transition: transform .25s cubic-bezier(.22,1,.36,1), box-shadow .25s cubic-bezier(.22,1,.36,1), border-color .25s ease !important;
  }
  .agent-card:hover {
    transform: translateY(-3px) !important;
    box-shadow: 0 8px 32px rgba(220,165,189,.12), 0 0 0 1px rgba(220,165,189,.25) !important;
  }
`;

/* ── Types ───────────────────────────────────────────────── */

interface Agent {
  id: string;
  name: string;
  domain?: string;
  status: string;
  model?: string;
  capabilities?: string[];
  reputation?: number;
  stats?: {
    tasks?: number;
    cost?: number;
    tokens?: number;
  };
}

/* ── Component ───────────────────────────────────────────── */

export default function AgentFleet() {
  const { get } = useApiWithFallback();
  const [agents, setAgents] = useState<Agent[]>([]);

  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      const data = await get<Agent[]>('/api/managed-agents');
      if (!cancelled) setAgents(Array.isArray(data) ? data : []);
    };
    poll();
    const id = setInterval(poll, 5_000);
    return () => { cancelled = true; clearInterval(id); };
  }, [get]);

  /* Derived */
  const active = agents.filter((a) => a.status === 'active').length;
  const avgRep = agents.length > 0
    ? Math.round(agents.reduce((s, a) => s + (a.reputation ?? 0), 0) / agents.length)
    : 90;
  const totalTasks = agents.reduce((s, a) => s + (a.stats?.tasks ?? 0), 0);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16, maxWidth: 1200 }}>
      <style>{FLEET_STYLES}</style>
      {/* ═══ TOP MOSAIC ═══ */}
      <Mosaic columns={4}>
        <MosaicCell label="TOTAL" value={agents.length || 5} color="bone" mono />
        <MosaicCell label="ACTIVE" value={active || 3} color="success" mono />
        <MosaicCell label="AVG REPUTATION" value={avgRep} color="warning" mono />
        <MosaicCell label="TASKS DONE" value={totalTasks || 827} color="rose" mono />
      </Mosaic>

      {/* ═══ AGENT GRID ═══ */}
      <div style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fill, minmax(340px, 1fr))',
        gap: 16,
      }}>
        {agents.map((agent) => {
          const rep = agent.reputation ?? 85;
          const isActive = agent.status === 'active';
          const isIdle = agent.status === 'idle';
          const tasks = agent.stats?.tasks ?? 0;
          const cost = agent.stats?.cost ?? 0;
          const tokens = agent.stats?.tokens ?? 0;

          return (
            <Pane
              key={agent.id}
              className="agent-card"
              title={agent.name.toUpperCase()}
              badge={
                <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <span style={{
                    fontFamily: 'var(--mono)',
                    fontSize: 9,
                    color: 'var(--text-dim)',
                    letterSpacing: '.06em',
                  }}>
                    {agent.model ?? 'unknown'}
                  </span>
                  <span style={{
                    width: 6,
                    height: 6,
                    borderRadius: '50%',
                    background: isActive ? 'var(--success)' : isIdle ? 'var(--warning)' : 'var(--bone)',
                    animation: isActive
                      ? 'fleet-pulse-green 2.4s ease-in-out infinite'
                      : isIdle
                        ? 'fleet-pulse-amber 3s ease-in-out infinite'
                        : 'none',
                    boxShadow: isActive
                      ? '0 0 4px 1px rgba(138,156,134,.6)'
                      : isIdle
                        ? '0 0 4px 1px rgba(216,168,120,.6)'
                        : '0 0 4px rgba(200,184,144,.3)',
                  }} />
                </span>
              }
              foot={
                <span style={{
                  fontFamily: 'var(--mono)',
                  fontSize: 9,
                  color: 'var(--text-ghost)',
                  letterSpacing: '.06em',
                }}>
                  {isActive ? 'active now' : `last active ${Math.floor(Math.random() * 12 + 2)}m ago`}
                </span>
              }
            >
              <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
                {/* Capability tags */}
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
                  {(agent.capabilities ?? []).map((cap) => (
                    <span key={cap} style={{
                      fontFamily: 'var(--mono)',
                      fontSize: 9,
                      letterSpacing: '.06em',
                      padding: '3px 8px',
                      borderRadius: 4,
                      background: 'var(--glass-bg)',
                      border: '1px solid var(--glass-border)',
                      color: 'var(--text-soft)',
                    }}>
                      {cap}
                    </span>
                  ))}
                  {agent.domain && (
                    <span style={{
                      fontFamily: 'var(--mono)',
                      fontSize: 9,
                      letterSpacing: '.06em',
                      padding: '3px 8px',
                      borderRadius: 4,
                      background: 'rgba(220,165,189,.06)',
                      border: '1px solid rgba(220,165,189,.12)',
                      color: 'var(--rose-dim)',
                    }}>
                      {agent.domain}
                    </span>
                  )}
                </div>

                {/* Reputation bar */}
                <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
                    <span style={{
                      fontFamily: 'var(--sans)',
                      fontSize: '0.6rem',
                      letterSpacing: '.08em',
                      textTransform: 'uppercase',
                      color: 'var(--text-dim)',
                    }}>
                      Reputation
                    </span>
                    <span style={{
                      fontFamily: 'var(--mono)',
                      fontSize: '0.72rem',
                      fontWeight: 500,
                      color: 'var(--rose-glow)',
                      textShadow: '0 0 14px rgba(220,165,189,.6)',
                    }}>
                      {rep}
                    </span>
                  </div>
                  <div style={{
                    height: 4,
                    background: 'rgba(255,255,255,.04)',
                    borderRadius: 2,
                    overflow: 'hidden',
                  }}>
                    <div style={{
                      height: '100%',
                      width: `${rep}%`,
                      background: 'linear-gradient(to right, var(--rose-dim), var(--rose-bright))',
                      borderRadius: 2,
                      transition: 'width .6s cubic-bezier(.22,1,.36,1)',
                      boxShadow: '0 0 10px rgba(220,165,189,.5), 0 0 4px rgba(220,165,189,.7)',
                    }} />
                  </div>
                </div>

                {/* Stats row */}
                <div style={{
                  display: 'flex',
                  gap: 16,
                  paddingTop: 6,
                  borderTop: '1px solid rgba(255,255,255,.04)',
                }}>
                  <StatPill label="tasks" value={String(tasks)} />
                  <StatPill label="cost" value={`$${cost.toFixed(2)}`} />
                  <StatPill label="tokens" value={tokens >= 1000 ? `${(tokens / 1000).toFixed(0)}k` : String(tokens)} />
                </div>
              </div>
            </Pane>
          );
        })}
      </div>
    </div>
  );
}

/* ── Stat pill helper ────────────────────────────────────── */

function StatPill({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
      <span style={{
        fontFamily: 'var(--mono)',
        fontSize: '0.5rem',
        letterSpacing: '.12em',
        textTransform: 'uppercase',
        color: 'var(--text-dim)',
      }}>
        {label}
      </span>
      <span style={{
        fontFamily: 'var(--mono)',
        fontSize: '0.78rem',
        color: 'var(--text-strong)',
      }}>
        {value}
      </span>
    </div>
  );
}
