import { Cell } from './Cell';
import { Badge } from '../design/Badge';

type AgentStatus = 'idle' | 'active' | 'done' | 'error';
type CellStatus = 'idle' | 'active' | 'success' | 'error' | 'blocked';

const STATUS_MAP: Record<AgentStatus, CellStatus> = {
  idle: 'idle',
  active: 'active',
  done: 'success',
  error: 'error',
};

interface AgentCellProps {
  agent: {
    name: string;
    role?: string;
    status: AgentStatus;
    taskCount?: number;
    cost?: number;
    model?: string;
  };
  onClick?: () => void;
}

function formatCost(cost: number): string {
  return cost < 0.01 ? `$${cost.toFixed(4)}` : `$${cost.toFixed(2)}`;
}

export function AgentCell({ agent, onClick }: AgentCellProps) {
  return (
    <Cell
      status={STATUS_MAP[agent.status]}
      identity="AGENT"
      onClick={onClick}
    >
      <div style={{ fontSize: 'var(--text-sm)', fontWeight: 600, color: 'var(--text-primary)' }}>
        {agent.name}
      </div>
      {agent.role && (
        <div style={{
          fontFamily: 'var(--mono)',
          fontSize: 'var(--text-sm)',
          textTransform: 'uppercase',
          letterSpacing: '0.08em',
          color: 'var(--text-dim)',
          marginTop: '2px',
        }}>
          {agent.role}
        </div>
      )}
      <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--sp-1)', marginTop: 'var(--sp-2)', flexWrap: 'wrap' }}>
        {agent.taskCount != null && (
          <Badge>{agent.taskCount} task{agent.taskCount !== 1 ? 's' : ''}</Badge>
        )}
        {agent.cost != null && (
          <Badge>{formatCost(agent.cost)}</Badge>
        )}
        {agent.model && <Badge variant="info">{agent.model}</Badge>}
      </div>
    </Cell>
  );
}
