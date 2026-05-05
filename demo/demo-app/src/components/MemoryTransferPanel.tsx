import { useState, useMemo } from 'react';
import { useEventStreamContext, useContextEventSubscription } from '../contexts/EventStreamContext';
import { useInferenceCosts } from '../hooks/useOperationEvents';
import './MemoryTransferPanel.css';

interface MemoryTransferPanelProps {
  /** Operation ID for the cold run */
  coldOpId?: string | null;
  /** Operation ID for the warm run */
  warmOpId?: string | null;
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

export default function MemoryTransferPanel({
  coldOpId = null,
  warmOpId = null,
  isRunning = false,
}: MemoryTransferPanelProps) {
  const { connected } = useEventStreamContext();
  const coldCosts = useInferenceCosts(coldOpId);
  const warmCosts = useInferenceCosts(warmOpId);
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

  const hasBothCosts = coldCosts.totalCost > 0 && warmCosts.totalCost > 0;
  const costDelta = hasBothCosts
    ? ((coldCosts.totalCost - warmCosts.totalCost) / coldCosts.totalCost) * 100
    : null;

  const ingestedCount = knowledgeEvents.filter((e) => e.type === 'ingested').length;
  const consumedCount = knowledgeEvents.filter((e) => e.type === 'consumed').length;

  type Phase = 'cold' | 'ingest' | 'warm' | 'delta';
  const currentPhase: Phase = useMemo(() => {
    if (warmCosts.calls > 0) return hasBothCosts ? 'delta' : 'warm';
    if (ingestedCount > 0) return 'ingest';
    if (coldCosts.calls > 0) return 'cold';
    return 'cold';
  }, [coldCosts.calls, warmCosts.calls, ingestedCount, hasBothCosts]);

  const phases: { id: Phase; label: string }[] = [
    { id: 'cold', label: 'Cold' },
    { id: 'ingest', label: 'Ingest' },
    { id: 'warm', label: 'Warm' },
    { id: 'delta', label: 'Delta' },
  ];

  const phaseOrder: Phase[] = ['cold', 'ingest', 'warm', 'delta'];
  const currentIdx = phaseOrder.indexOf(currentPhase);

  const panelState = isRunning
    ? 'running'
    : coldCosts.calls > 0 || warmCosts.calls > 0 ? 'data' : 'pending';

  return (
    <section className="memory-panel" aria-label="Memory transfer">
      <div className="memory-panel-header">
        <span className="memory-panel-title">Knowledge Transfer</span>
        <span className={`memory-panel-live ${connected ? 'connected' : ''}`}>
          {panelState === 'pending' ? (connected ? 'armed' : 'offline') : 'live'}
        </span>
      </div>

      <div className="memory-panel-phases">
        {phases.map((phase, i) => {
          const status = i < currentIdx ? 'done' : i === currentIdx && (isRunning || coldCosts.calls > 0) ? 'active' : '';
          return (
            <div key={phase.id} className={`memory-panel-phase${status ? ` memory-panel-phase--${status}` : ''}`}>
              <div className="memory-panel-phase-dot" />
              <div className="memory-panel-phase-label">{phase.label}</div>
            </div>
          );
        })}
      </div>

      <div className="memory-panel-columns">
        <div className="memory-panel-column memory-panel-column--cold">
          <div className="memory-panel-column-label">Cold Run</div>
          <div className="memory-panel-metric">
            <span className="memory-panel-metric-label">Cost</span>
            <span className={`memory-panel-metric-value${coldCosts.totalCost <= 0 ? ' memory-panel-metric-value--empty' : ''}`}>
              {fmtCost(coldCosts.totalCost)}
            </span>
          </div>
          <div className="memory-panel-metric">
            <span className="memory-panel-metric-label">Tokens</span>
            <span className={`memory-panel-metric-value${coldCosts.totalTokens <= 0 ? ' memory-panel-metric-value--empty' : ''}`}>
              {fmtTokens(coldCosts.totalTokens)}
            </span>
          </div>
          <div className="memory-panel-metric">
            <span className="memory-panel-metric-label">Calls</span>
            <span className={`memory-panel-metric-value${coldCosts.calls <= 0 ? ' memory-panel-metric-value--empty' : ''}`}>
              {coldCosts.calls > 0 ? String(coldCosts.calls) : '--'}
            </span>
          </div>
        </div>

        <div className="memory-panel-column memory-panel-column--warm">
          <div className="memory-panel-column-label">Warm Run</div>
          <div className="memory-panel-metric">
            <span className="memory-panel-metric-label">Cost</span>
            <span className={`memory-panel-metric-value${warmCosts.totalCost <= 0 ? ' memory-panel-metric-value--empty' : ''}`}>
              {fmtCost(warmCosts.totalCost)}
            </span>
          </div>
          <div className="memory-panel-metric">
            <span className="memory-panel-metric-label">Tokens</span>
            <span className={`memory-panel-metric-value${warmCosts.totalTokens <= 0 ? ' memory-panel-metric-value--empty' : ''}`}>
              {fmtTokens(warmCosts.totalTokens)}
            </span>
          </div>
          <div className="memory-panel-metric">
            <span className="memory-panel-metric-label">Calls</span>
            <span className={`memory-panel-metric-value${warmCosts.calls <= 0 ? ' memory-panel-metric-value--empty' : ''}`}>
              {warmCosts.calls > 0 ? String(warmCosts.calls) : '--'}
            </span>
          </div>
        </div>
      </div>

      <div className="memory-panel-transfer">
        <div className="memory-panel-transfer-title">Efficiency Gain</div>
        <div className={`memory-panel-transfer-value${
          costDelta !== null && costDelta > 0 ? ' memory-panel-transfer-value--savings' : ''
        }`}>
          {costDelta !== null ? `${costDelta > 0 ? '' : '+'}${Math.abs(costDelta).toFixed(0)}%` : '--'}
        </div>
        <div className="memory-panel-transfer-label">
          {costDelta !== null
            ? costDelta > 0 ? 'warm run cheaper via knowledge reuse' : 'warm run needed more work'
            : 'run both to compare'}
        </div>
      </div>

      {knowledgeEvents.length > 0 && (
        <div className="memory-panel-knowledge">
          <div className="memory-panel-knowledge-title">
            Knowledge Events ({ingestedCount} ingested, {consumedCount} consumed)
          </div>
          {knowledgeEvents.slice(-5).reverse().map((ev) => (
            <div key={ev.id} className="memory-panel-knowledge-row">
              <span className={`memory-panel-knowledge-type memory-panel-knowledge-type--${ev.type}`}>
                {ev.type}
              </span>
              <span className="memory-panel-knowledge-content">
                {ev.topic || 'knowledge entry'}
              </span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
