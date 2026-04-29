/**
 * ChainIntelPanel — composite sidebar panel for the Chain Intelligence demo.
 *
 * Stacks all four chain-related components vertically:
 *   1. KnowledgeFlowPanel (insight flow between agents)
 *   2. ChainActivityPanel + LivePositionsPanel side-by-side
 *   3. EfficiencyBar (bottom metrics)
 */

import KnowledgeFlowPanel, {
  type InsightEvent as KFInsight,
  type AgentInfo,
} from './KnowledgeFlowPanel';
import ChainActivityPanel, { type BlockData } from './ChainActivityPanel';
import LivePositionsPanel, { type AgentPosition } from './LivePositionsPanel';
import EfficiencyBar, { type EfficiencyMetric } from './EfficiencyBar';
import './ChainIntelPanel.css';

export interface ChainIntelPanelProps {
  /** Left agent (Alpha / Yield Scout) info for the knowledge graph */
  leftAgent: AgentInfo;
  /** Right agent (Beta / Risk Hedger) info for the knowledge graph */
  rightAgent: AgentInfo;
  /** Insight events from mirage WebSocket */
  insights: KFInsight[];
  /** Recent blocks with transactions */
  blocks: BlockData[];
  /** Agent portfolio positions */
  positions: AgentPosition[];
  /** Efficiency metrics for the bottom bar */
  metrics: EfficiencyMetric[];
  /** Whether mirage is connected */
  mirageConnected: boolean;
}

export default function ChainIntelPanel({
  leftAgent,
  rightAgent,
  insights,
  blocks,
  positions,
  metrics,
  mirageConnected,
}: ChainIntelPanelProps) {
  if (!mirageConnected) {
    return (
      <div className="chain-intel-panel">
        <div className="chain-intel-offline">
          <div className="chain-intel-offline-icon">
            <span className="chain-intel-offline-ring" />
            <span className="chain-intel-offline-ring chain-intel-offline-ring-2" />
          </div>
          <div className="chain-intel-offline-title">Mirage not connected</div>
          <div className="chain-intel-offline-hint">
            Start mirage-rs with chain features to enable this demo:
          </div>
          <pre className="chain-intel-offline-cmd">
            {`cargo run -p mirage-rs --features chain -- \\
  --rpc-url https://eth.llamarpc.com \\
  --block-interval-ms 2000`}
          </pre>
          <div className="chain-intel-offline-hint">
            The panel will update automatically once mirage is reachable.
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="chain-intel-panel chain-intel-connected">
      {/* Top: Knowledge flow visualization */}
      <div className="chain-intel-section chain-intel-knowledge">
        <KnowledgeFlowPanel
          leftAgent={leftAgent}
          rightAgent={rightAgent}
          insights={insights}
          mode="chain"
        />
      </div>

      {/* Section connector */}
      <div className="chain-intel-connector">
        <div className="chain-intel-connector-line" />
        <div className="chain-intel-connector-energy" />
      </div>

      {/* Middle: Chain activity + Live positions side-by-side */}
      <div className="chain-intel-section chain-intel-middle">
        <div className="chain-intel-col">
          <ChainActivityPanel blocks={blocks} maxBlocks={10} />
        </div>
        <div className="chain-intel-col">
          <LivePositionsPanel agents={positions} />
        </div>
      </div>

      {/* Section connector */}
      <div className="chain-intel-connector">
        <div className="chain-intel-connector-line" />
        <div className="chain-intel-connector-energy" />
      </div>

      {/* Bottom: Efficiency metrics */}
      <div className="chain-intel-section chain-intel-efficiency">
        <EfficiencyBar metrics={metrics} />
      </div>
    </div>
  );
}
