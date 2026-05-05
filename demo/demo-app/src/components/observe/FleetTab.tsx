import { SplitView } from '../layout/SplitView';
import { AgentCell } from '../cells/AgentCell';
import { TimelineCanvas, type TimelineEntry } from './TimelineCanvas';
import Pane from '../Pane';
import './FleetTab.css';

/* ── Types ────────────────────────────────────────────── */

type AgentStatus = 'idle' | 'active' | 'done' | 'error';

export interface FleetAgent {
  name: string;
  role?: string;
  status: AgentStatus;
  taskCount?: number;
  cost?: number;
  model?: string;
}

export interface FleetTabProps {
  agents: FleetAgent[];
  timelineEntries: TimelineEntry[];
  onAgentClick?: (name: string) => void;
}

/* ── Component ────────────────────────────────────────── */

/**
 * Fleet tab: split-view with timeline canvas (left) and agent cards (right).
 * Extracted from Explorer.tsx episode/agent sections.
 */
export function FleetTab({ agents, timelineEntries, onAgentClick }: FleetTabProps) {
  return (
    <div className="fleet-tab">
      <SplitView
        left={
          <Pane title="Timeline" flat>
            <TimelineCanvas entries={timelineEntries} height={280} />
          </Pane>
        }
        right={
          <Pane title="Agents" flat>
            <div className="fleet-tab__agents">
              {agents.map((a) => (
                <AgentCell
                  key={a.name}
                  agent={a}
                  onClick={onAgentClick ? () => onAgentClick(a.name) : undefined}
                />
              ))}
              {agents.length === 0 && (
                <p className="fleet-tab__empty">No agents active</p>
              )}
            </div>
          </Pane>
        }
        defaultSplit={55}
      />
    </div>
  );
}
