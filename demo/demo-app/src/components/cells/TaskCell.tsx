import { Cell } from './Cell';
import { Badge } from '../design/Badge';
import { GateBar, type GateResult } from '../design/GateBar';

type TaskStatus = 'pending' | 'running' | 'done' | 'failed' | 'blocked';
type CellStatus = 'idle' | 'active' | 'success' | 'error' | 'blocked';

const STATUS_MAP: Record<TaskStatus, CellStatus> = {
  pending: 'idle',
  running: 'active',
  done: 'success',
  failed: 'error',
  blocked: 'blocked',
};

interface TaskCellProps {
  task: {
    id: string;
    title: string;
    status: TaskStatus;
    tier?: string;
    model?: string;
    gates?: GateResult[];
    cost?: number;
    duration?: number;
  };
  onClick?: () => void;
}

function formatCost(cost: number): string {
  return cost < 0.01 ? `$${cost.toFixed(4)}` : `$${cost.toFixed(2)}`;
}

export function TaskCell({ task, onClick }: TaskCellProps) {
  return (
    <Cell
      status={STATUS_MAP[task.status]}
      identity="TASK"
      onClick={onClick}
    >
      <div style={{ fontSize: 'var(--text-sm)', color: 'var(--text-primary)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        {task.title}
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: '6px', marginTop: '8px', flexWrap: 'wrap' }}>
        {task.tier && <Badge>{task.tier}</Badge>}
        {task.model && <Badge variant="info">{task.model}</Badge>}
        {task.cost != null && (
          <Badge>{formatCost(task.cost)}</Badge>
        )}
      </div>
      {task.gates && task.gates.length > 0 && (
        <div style={{ marginTop: '8px' }}>
          <GateBar gates={task.gates} />
        </div>
      )}
    </Cell>
  );
}
