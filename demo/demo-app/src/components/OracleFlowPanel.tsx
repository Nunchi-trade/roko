import { useState } from 'react';
import { useEventStreamContext, useContextEventSubscription } from '../contexts/EventStreamContext';
import { useInferenceCosts } from '../hooks/useOperationEvents';
import './OracleFlowPanel.css';

interface OracleFlowPanelProps {
  /** Operation ID for the data-agent run */
  dataOpId?: string | null;
  /** Operation ID for the strategy-agent run */
  strategyOpId?: string | null;
  /** Whether the chain check has been performed */
  chainChecked?: boolean;
  isRunning?: boolean;
}

interface KnowledgeEvent {
  id: string;
  type: 'ingested' | 'consumed';
  topic?: string;
  timestamp: number;
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

function readStr(obj: Record<string, unknown>, keys: string[]): string {
  for (const k of keys) {
    const v = obj[k];
    if (typeof v === 'string' && v.length > 0) return v;
  }
  for (const nest of ['data', 'event'] as const) {
    const sub = obj[nest];
    if (!isRecord(sub)) continue;
    for (const k of keys) {
      const v = sub[k];
      if (typeof v === 'string' && v.length > 0) return v;
    }
  }
  return '';
}

function fmtCost(n: number): string {
  if (n <= 0) return '--';
  if (n < 0.001) return `$${n.toFixed(4)}`;
  if (n < 0.01) return `$${n.toFixed(3)}`;
  return `$${n.toFixed(2)}`;
}

function fmtTokens(n: number): string {
  if (n <= 0) return '--';
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}

type FlowStep = 'connect' | 'scan' | 'write' | 'recommend';

const FLOW_STEPS: { id: FlowStep; label: string; desc: string; icon: string }[] = [
  { id: 'connect', label: 'Connect', desc: 'Verify local Ethereum fork', icon: '1' },
  { id: 'scan', label: 'Scan', desc: 'Query DeFi lending rates', icon: '2' },
  { id: 'write', label: 'Write', desc: 'Store analysis to knowledge', icon: '3' },
  { id: 'recommend', label: 'Recommend', desc: 'USDC allocation strategy', icon: '4' },
];

export default function OracleFlowPanel({
  dataOpId = null,
  strategyOpId = null,
  chainChecked = false,
  isRunning = false,
}: OracleFlowPanelProps) {
  const { connected } = useEventStreamContext();
  const dataCosts = useInferenceCosts(dataOpId);
  const strategyCosts = useInferenceCosts(strategyOpId);
  const [knowledgeEvents, setKnowledgeEvents] = useState<KnowledgeEvent[]>([]);

  useContextEventSubscription(
    ['knowledge_ingested', 'knowledge_consumed'],
    (event: unknown) => {
      if (!isRecord(event)) return;
      const type = typeof event.type === 'string' ? event.type : '';
      const mapped: 'ingested' | 'consumed' =
        type === 'knowledge_consumed' ? 'consumed' : 'ingested';
      const topic = readStr(event, ['topic', 'key', 'title', 'path']);

      setKnowledgeEvents((prev) => [
        ...prev.slice(-19),
        {
          id: `${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
          type: mapped,
          topic: topic || undefined,
          timestamp: Date.now(),
        },
      ]);
    },
  );

  // Determine current flow step
  let currentStep: FlowStep = 'connect';
  if (strategyCosts.calls > 0) currentStep = 'recommend';
  else if (knowledgeEvents.some((e) => e.type === 'ingested')) currentStep = 'write';
  else if (dataCosts.calls > 0) currentStep = 'scan';
  else if (chainChecked) currentStep = 'connect';

  const stepOrder: FlowStep[] = ['connect', 'scan', 'write', 'recommend'];
  const currentIdx = stepOrder.indexOf(currentStep);
  const hasAnyData = chainChecked || dataCosts.calls > 0 || strategyCosts.calls > 0;

  const panelState = isRunning
    ? 'running'
    : hasAnyData ? 'data' : 'pending';

  const totalCost = dataCosts.totalCost + strategyCosts.totalCost;
  const totalTokens = dataCosts.totalTokens + strategyCosts.totalTokens;
  const totalCalls = dataCosts.calls + strategyCosts.calls;

  return (
    <section className="oracle-panel" aria-label="Oracle flow">
      <div className="oracle-panel-header">
        <span className="oracle-panel-title">Oracle Flow</span>
        <span className={`oracle-panel-live ${connected ? 'connected' : ''}`}>
          {panelState === 'pending' ? (connected ? 'armed' : 'offline') : 'live'}
        </span>
      </div>

      <div className="oracle-panel-flow">
        {FLOW_STEPS.map((step, i) => {
          const status = hasAnyData
            ? i < currentIdx ? 'done' : i === currentIdx ? 'active' : ''
            : '';
          return (
            <div key={step.id}>
              {i > 0 && <div className="oracle-panel-step-connector" />}
              <div className={`oracle-panel-step${status ? ` oracle-panel-step--${status}` : ''}`}>
                <div className="oracle-panel-step-icon">
                  {status === 'done' ? '\u2713' : step.icon}
                </div>
                <div className="oracle-panel-step-body">
                  <div className="oracle-panel-step-label">{step.label}</div>
                  <div className="oracle-panel-step-desc">{step.desc}</div>
                </div>
              </div>
            </div>
          );
        })}
      </div>

      <div className="oracle-panel-metrics">
        <div className="oracle-panel-metric">
          <div className="oracle-panel-metric-label">Total Cost</div>
          <div className={`oracle-panel-metric-value${totalCost <= 0 ? ' oracle-panel-metric-value--empty' : ''}`}>
            {fmtCost(totalCost)}
          </div>
        </div>
        <div className="oracle-panel-metric">
          <div className="oracle-panel-metric-label">Tokens</div>
          <div className={`oracle-panel-metric-value${totalTokens <= 0 ? ' oracle-panel-metric-value--empty' : ''}`}>
            {fmtTokens(totalTokens)}
          </div>
        </div>
        <div className="oracle-panel-metric">
          <div className="oracle-panel-metric-label">Data Calls</div>
          <div className={`oracle-panel-metric-value${dataCosts.calls <= 0 ? ' oracle-panel-metric-value--empty' : ''}`}>
            {dataCosts.calls > 0 ? String(dataCosts.calls) : '--'}
          </div>
        </div>
        <div className="oracle-panel-metric">
          <div className="oracle-panel-metric-label">Total Calls</div>
          <div className={`oracle-panel-metric-value${totalCalls <= 0 ? ' oracle-panel-metric-value--empty' : ''}`}>
            {totalCalls > 0 ? String(totalCalls) : '--'}
          </div>
        </div>
      </div>

      {knowledgeEvents.length > 0 && (
        <div className="oracle-panel-knowledge">
          <div className="oracle-panel-knowledge-title">
            Knowledge Flow
          </div>
          {knowledgeEvents.slice(-5).reverse().map((ev) => (
            <div key={ev.id} className="oracle-panel-knowledge-row">
              <span className={`oracle-panel-knowledge-type oracle-panel-knowledge-type--${ev.type}`}>
                {ev.type}
              </span>
              <span className="oracle-panel-knowledge-content">
                {ev.topic || 'knowledge entry'}
              </span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
