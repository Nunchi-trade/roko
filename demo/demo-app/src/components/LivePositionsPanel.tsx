import PhosphorNumber from './PhosphorNumber';
import Pane from './Pane';
import './LivePositionsPanel.css';

/* ── Types ──────────────────────────────────────────────── */

export interface AgentPosition {
  name: string;
  address: string;
  color: string; // rose or sage
  balances: { token: string; amount: number; decimals?: number }[];
  keyMetric: { label: string; value: string }; // "APR: 7.2%" or "HF: 2.31"
  strategy?: string;
}

interface LivePositionsPanelProps {
  agents: AgentPosition[];
}

/* ── Helpers ────────────────────────────────────────────── */

function truncateAddress(addr: string): string {
  if (addr.length <= 10) return addr;
  return `${addr.slice(0, 6)}...${addr.slice(-4)}`;
}

function formatBalance(amount: number, decimals = 4): string {
  return amount.toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: decimals,
  });
}

function colorVar(color: string): string {
  if (color === 'sage') return 'var(--success)';
  return 'var(--rose-bright)';
}

/* ── AgentCard ──────────────────────────────────────────── */

function AgentCard({ agent }: { agent: AgentPosition }) {
  return (
    <div className="agent-card">
      <div className="agent-card-header">
        <span className="agent-card-name" style={{ color: colorVar(agent.color) }}>
          {agent.name}
        </span>
        <span className="agent-card-address">{truncateAddress(agent.address)}</span>
      </div>

      <div className="agent-card-balances">
        {agent.balances.map((b) => (
          <div className="balance-row" key={b.token}>
            <span className="balance-token">{b.token}</span>
            <PhosphorNumber
              className="balance-amount"
              value={b.amount}
              format={(n) => formatBalance(n, b.decimals ?? 4)}
            />
          </div>
        ))}
      </div>

      <div className="agent-card-metric">
        <span className="metric-label">{agent.keyMetric.label}</span>
        <span className="metric-value">{agent.keyMetric.value}</span>
      </div>

      {agent.strategy && (
        <div className="agent-card-strategy">{agent.strategy}</div>
      )}
    </div>
  );
}

/* ── Panel ──────────────────────────────────────────────── */

export default function LivePositionsPanel({ agents }: LivePositionsPanelProps) {
  return (
    <Pane title="Live Positions" badge={<span>{agents.length} agents</span>}>
      <div className="live-positions">
        {agents.map((a) => (
          <AgentCard key={a.address} agent={a} />
        ))}
      </div>
    </Pane>
  );
}
