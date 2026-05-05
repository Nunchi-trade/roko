import type { GateEntry } from './GateVerdictCard';
import type { PipelineDemoState, PipelineScenarioExample } from '../lib/prd-pipeline-types';
import type { InsightEvent, AgentInfo } from './KnowledgeFlowPanel';
import type { EfficiencyMetric } from './EfficiencyBar';
import type { BlockData } from './ChainActivityPanel';
import type { AgentPosition } from './LivePositionsPanel';
import type { ServerStatus } from '../hooks/useServerHealth';
import type { LearningStats } from '../hooks/useLearningStats';
import type { HandoffEntry } from '../hooks/useAgentHandoffs';

import Pane from './Pane';
import Mosaic, { MosaicCell } from './Mosaic';
import Timeline from './Timeline';
import CommandLog from './CommandLog';
import GateVerdictCard from './GateVerdictCard';
import PrdPipelinePanel from './PrdPipelinePanel';
import KnowledgeFlowPanel from './KnowledgeFlowPanel';
import EfficiencyBar from './EfficiencyBar';
import ChainIntelPanel from './ChainIntelPanel';
import ISFRPanel from './ISFRPanel';
import RevealWhen from './RevealWhen';
import InferenceTracePanel from './InferenceTracePanel';
import type { InferenceCall, InferenceTraceTotals } from '../hooks/useInferenceTrace';
import { ConfidenceMeter, ModelSlot, CrystallizeTransition } from './inference';
import { AgentHandoff } from './agent';

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export interface SidebarRendererProps {
  scenarioId: string;
  isRunning: boolean;
  scenarioComplete: boolean;

  // Timeline
  timelineSteps: { label: string; detail?: string; status: 'active' | 'pending' | 'done' }[];

  // Stats
  stats: { model: string; cost: string; tokens: string; time: string };
  hasStats: boolean;
  inferenceModel: string;
  inferenceTier: 'T0' | 'T1' | 'T2';

  // Gates
  gates: { name: string; status: 'pass' | 'fail' | 'pending' }[];
  gateEntries: GateEntry[];
  allGatesPass: boolean;

  // Log
  logEntries: { ts: string; text: string; type?: 'info' | 'success' | 'error' }[];

  // Pipeline (PRD)
  pipeline: PipelineDemoState;
  pipelineExamples: PipelineScenarioExample[];
  pipelineExampleId: string;
  onSelectExample: (id: string) => void;
  onRun: () => void;
  serverHealth: ServerStatus;
  learningStats: LearningStats;

  // Handoffs
  handoffs: HandoffEntry[];
  activeHandoff: HandoffEntry | null;

  // Knowledge transfer
  kfInsights: InsightEvent[];
  kfLeftAgent: AgentInfo;
  kfRightAgent: AgentInfo;
  kfMetrics: EfficiencyMetric[];
  hasKfMetrics: boolean;

  // Chain intelligence
  ciInsights: InsightEvent[];
  ciBlocks: BlockData[];
  ciPositions: AgentPosition[];
  ciMetrics: EfficiencyMetric[];
  ciLeftAgent: AgentInfo;
  ciRightAgent: AgentInfo;
  chainConnected: boolean;

  // Inference trace
  traceCalls?: InferenceCall[];
  traceTotals?: InferenceTraceTotals;
  traceCostSeries?: number[];
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export default function SidebarRenderer(props: SidebarRendererProps) {
  const {
    scenarioId,
    isRunning,
    scenarioComplete,
    timelineSteps,
    stats,
    hasStats,
    inferenceModel,
    inferenceTier,
    gates,
    gateEntries,
    allGatesPass,
    logEntries,
    pipeline,
    pipelineExamples,
    pipelineExampleId,
    onSelectExample,
    onRun,
    serverHealth,
    learningStats,
    handoffs,
    activeHandoff,
    kfInsights,
    kfLeftAgent,
    kfRightAgent,
    kfMetrics,
    hasKfMetrics,
    ciInsights,
    ciBlocks,
    ciPositions,
    ciMetrics,
    ciLeftAgent,
    ciRightAgent,
    chainConnected,
    traceCalls = [],
    traceTotals = { cost: 0, tokens: 0, calls: 0, avgLatencyMs: 0 },
    traceCostSeries = [],
  } = props;

  if (scenarioId === 'prd-pipeline') {
    return (
      <>
        <PrdPipelinePanel
          state={pipeline}
          examples={pipelineExamples}
          selectedExampleId={pipelineExampleId}
          onSelectExample={onSelectExample}
          selectorDisabled={isRunning}
          onRun={onRun}
          isRunning={isRunning}
          serverHealth={serverHealth}
          learningStats={learningStats}
        />

        <RevealWhen visible={handoffs.length > 0} mode="slide-up">
          <Pane title="AGENT FLOW" flat>
            <div className="demo-sidebar-agent-flow">
              {handoffs.slice(-3).map((h) => (
                <AgentHandoff
                  key={h.id}
                  from={h.from}
                  to={h.to}
                  status={h.status}
                  direction="forward"
                  label={h.label}
                  compact
                />
              ))}
            </div>
          </Pane>
        </RevealWhen>

        <RevealWhen visible={learningStats.totalDecisions > 0} mode="blur">
          <div className="demo-sidebar-confidence">
            <ConfidenceMeter
              confidence={learningStats.routerConfidence}
              trend={learningStats.confidenceTrend}
              decisions={learningStats.totalDecisions}
              label="ROUTER CONFIDENCE"
              compact
            />
          </div>
        </RevealWhen>

        <InferenceTracePanel
          calls={traceCalls}
          totals={traceTotals}
          costSeries={traceCostSeries}
        />
      </>
    );
  }

  if (scenarioId === 'knowledge-transfer') {
    return (
      <>
        <RevealWhen visible={activeHandoff !== null} mode="slide-up">
          {activeHandoff && (
            <Pane title="HANDOFF" flat>
              <div className="demo-sidebar-handoff">
                <AgentHandoff
                  from={activeHandoff.from}
                  to={activeHandoff.to}
                  status={activeHandoff.status}
                  direction="forward"
                  label={activeHandoff.label}
                  compact
                />
              </div>
            </Pane>
          )}
        </RevealWhen>

        <RevealWhen visible={timelineSteps.length > 0} mode="slide-up">
          <Pane title="TIMELINE" flat>
            <Timeline steps={timelineSteps} />
          </Pane>
        </RevealWhen>

        <RevealWhen visible={kfInsights.length > 0} mode="scale">
          <KnowledgeFlowPanel
            leftAgent={kfLeftAgent}
            rightAgent={kfRightAgent}
            insights={kfInsights}
            mode="local"
          />
        </RevealWhen>

        <RevealWhen visible={hasKfMetrics} mode="blur">
          <EfficiencyBar metrics={kfMetrics} />
        </RevealWhen>

        <RevealWhen visible={gates.length > 0} mode="scale">
          <CrystallizeTransition active={allGatesPass}>
            <Pane title="GATES" flat>
              <div className="demo-sidebar-gates">
                <GateVerdictCard gates={gateEntries} compact />
              </div>
            </Pane>
          </CrystallizeTransition>
        </RevealWhen>

        <RevealWhen visible={logEntries.length > 0} mode="clip">
          <Pane title="LOG" flat>
            <CommandLog entries={logEntries} maxHeight="180px" />
          </Pane>
        </RevealWhen>

        <InferenceTracePanel
          calls={traceCalls}
          totals={traceTotals}
          costSeries={traceCostSeries}
        />
      </>
    );
  }

  if (scenarioId === 'chain-intelligence') {
    return (
      <>
        <RevealWhen visible={timelineSteps.length > 0} mode="slide-up">
          <Pane title="TIMELINE" flat>
            <Timeline steps={timelineSteps} />
          </Pane>
        </RevealWhen>

        <RevealWhen visible={ciInsights.length > 0 || chainConnected} mode="scale">
          <ChainIntelPanel
            leftAgent={ciLeftAgent}
            rightAgent={ciRightAgent}
            insights={ciInsights}
            blocks={ciBlocks}
            positions={ciPositions}
            metrics={ciMetrics}
            mirageConnected={chainConnected}
          />
        </RevealWhen>

        <RevealWhen visible={logEntries.length > 0} mode="clip">
          <Pane title="LOG" flat>
            <CommandLog entries={logEntries} maxHeight="140px" />
          </Pane>
        </RevealWhen>

        <InferenceTracePanel
          calls={traceCalls}
          totals={traceTotals}
          costSeries={traceCostSeries}
        />
      </>
    );
  }

  if (scenarioId === 'isfr-agents') {
    return (
      <>
        <RevealWhen visible={timelineSteps.length > 0} mode="slide-up">
          <Pane title="TIMELINE" flat>
            <Timeline steps={timelineSteps} />
          </Pane>
        </RevealWhen>

        <RevealWhen visible={ciInsights.length > 0 || chainConnected} mode="scale">
          <ISFRPanel
            insights={ciInsights}
            connected={chainConnected}
          />
        </RevealWhen>

        <RevealWhen visible={logEntries.length > 0} mode="clip">
          <Pane title="LOG" flat>
            <CommandLog entries={logEntries} maxHeight="140px" />
          </Pane>
        </RevealWhen>

        <InferenceTracePanel
          calls={traceCalls}
          totals={traceTotals}
          costSeries={traceCostSeries}
        />
      </>
    );
  }

  // Default sidebar
  return (
    <>
      <RevealWhen visible={timelineSteps.length > 0} mode="slide-up">
        <Pane title="TIMELINE" flat>
          <Timeline steps={timelineSteps} />
        </Pane>
      </RevealWhen>

      <RevealWhen visible={hasStats} mode="blur">
        <CrystallizeTransition active={scenarioComplete}>
          <div className="demo-stats-mosaic">
            <Mosaic columns={2}>
              <MosaicCell
                label="MODEL"
                mono
                color="rose"
                value={
                  inferenceModel !== '--'
                    ? <ModelSlot model={inferenceModel} tier={inferenceTier} size="sm" />
                    : '--'
                }
              />
              <MosaicCell label="COST" value={stats.cost} mono color="bone" />
              <MosaicCell label="TOKENS" value={stats.tokens} mono color="dream" />
              <MosaicCell label="TIME" value={stats.time} mono color="warning" />
            </Mosaic>
          </div>
        </CrystallizeTransition>
      </RevealWhen>

      <RevealWhen visible={learningStats.totalDecisions > 0} mode="blur">
        <div className="demo-sidebar-confidence">
          <ConfidenceMeter
            confidence={learningStats.routerConfidence}
            trend={learningStats.confidenceTrend}
            decisions={learningStats.totalDecisions}
            label="ROUTER CONFIDENCE"
            compact
          />
        </div>
      </RevealWhen>

      <RevealWhen visible={gates.length > 0} mode="scale">
        <CrystallizeTransition active={allGatesPass}>
          <Pane title="GATES" flat>
            <div className="demo-sidebar-gates">
              <GateVerdictCard gates={gateEntries} compact />
            </div>
          </Pane>
        </CrystallizeTransition>
      </RevealWhen>

      <RevealWhen visible={logEntries.length > 0} mode="clip">
        <Pane title="LOG" flat>
          <CommandLog entries={logEntries} maxHeight="240px" />
        </Pane>
      </RevealWhen>

      <InferenceTracePanel
        calls={traceCalls}
        totals={traceTotals}
        costSeries={traceCostSeries}
      />
    </>
  );
}
