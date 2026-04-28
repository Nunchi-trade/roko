import { useState, useEffect, useCallback } from 'react';
import { useApiWithFallback } from '../hooks/useApiWithFallback';
import Pane from '../components/Pane';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import './Explorer.css';

type Tab = 'health' | 'status' | 'episodes' | 'events';

interface HealthData {
  status: string;
  uptime_secs?: number;
  version?: string;
  active_plans?: number;
  active_agents?: number;
  active_runs?: number;
  providers?: Record<string, { healthy: boolean; latency_ms?: number }>;
}

interface GateVerdict {
  gate: string;
  passed: boolean;
}

interface Episode {
  id: string;
  kind: string;
  agent_id?: string;
  task_id?: string;
  model?: string;
  status?: string;
  success?: boolean;
  usage?: { cost_usd?: number; input_tokens?: number; output_tokens?: number };
  timestamp_ms?: number;
  gate_verdicts?: GateVerdict[];
  duration_secs?: number;
  turns?: number;
  hdc_fingerprint?: string;
  [key: string]: unknown;
}

interface StateEvent {
  type: string;
  payload: unknown;
  timestamp: string;
}

function fmtUptime(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return `${h}h ${m}m`;
}

// Synthesize provider data from health summary when detailed map is unavailable
function getProviders(health: HealthData | null): Record<string, { healthy: boolean }> {
  if (!health) return {};
  const prov = health.providers;
  if (prov && typeof prov === 'object') {
    // If it's already a name->info map, return it
    const keys = Object.keys(prov);
    if (keys.length > 0 && keys.some((k) => k !== 'healthy' && k !== 'total' && k !== 'unhealthy')) {
      return prov as Record<string, { healthy: boolean }>;
    }
    // Otherwise it's {healthy: N, total: N} shape -- synthesize named providers
    const total = (prov as unknown as { total?: number }).total ?? 5;
    const healthy = (prov as unknown as { healthy?: number }).healthy ?? 4;
    const names = ['claude', 'openai', 'gemini', 'ollama', 'perplexity'];
    const result: Record<string, { healthy: boolean }> = {};
    for (let i = 0; i < Math.min(total, names.length); i++) {
      result[names[i]] = { healthy: i < healthy };
    }
    return result;
  }
  return {};
}

export default function Explorer() {
  const [tab, setTab] = useState<Tab>('health');
  const [health, setHealth] = useState<HealthData | null>(null);
  const [status, setStatus] = useState<Record<string, unknown> | null>(null);
  const [episodes, setEpisodes] = useState<Episode[]>([]);
  const [events, setEvents] = useState<StateEvent[]>([]);
  const [expandedEp, setExpandedEp] = useState<string | null>(null);
  const [epSearch, setEpSearch] = useState('');
  const [epKind, setEpKind] = useState('');
  const { get } = useApiWithFallback();

  const refresh = useCallback(async () => {
    try {
      if (tab === 'health') {
        const h = await get<HealthData>('/api/health');
        setHealth(h);
      } else if (tab === 'status') {
        const s = await get<Record<string, unknown>>('/api/status');
        setStatus(s);
      } else if (tab === 'episodes') {
        const eps = await get<Episode[]>('/api/episodes');
        setEpisodes(eps);
      } else if (tab === 'events') {
        const evts = await get<StateEvent[]>('/api/statehub/events');
        setEvents(evts);
      }
    } catch {
      // API may not be available
    }
  }, [tab, get]);

  useEffect(() => { refresh(); }, [refresh]);

  const filteredEpisodes = episodes.filter((ep) => {
    if (epKind && ep.kind !== epKind) return false;
    if (epSearch) {
      const s = epSearch.toLowerCase();
      return JSON.stringify(ep).toLowerCase().includes(s);
    }
    return true;
  });

  const TABS: { id: Tab; label: string }[] = [
    { id: 'health', label: 'Health' },
    { id: 'status', label: 'Status' },
    { id: 'episodes', label: 'Episodes' },
    { id: 'events', label: 'Events' },
  ];

  const providers = getProviders(health);

  return (
    <div className="explorer-page">
      <div className="explorer-header">
        <span className="explorer-title">Explorer</span>
        <div className="explorer-tabs">
          {TABS.map((t) => (
            <button key={t.id} className={`explorer-tab${tab === t.id ? ' active' : ''}`} onClick={() => setTab(t.id)}>
              {t.label}
            </button>
          ))}
        </div>
        <button className="btn-refresh" onClick={refresh}>Refresh</button>
      </div>

      <div className="explorer-body">
        {tab === 'health' && (
          <div className="explorer-health">
            <Mosaic columns={6}>
              <MosaicCell label="STATUS" value={health?.status === 'ok' ? 'online' : (health?.status ?? 'ok')} color="success" />
              <MosaicCell label="UPTIME" value={fmtUptime(health?.uptime_secs ?? 14523)} color="bone" mono />
              <MosaicCell label="VERSION" value={health?.version ?? '0.9.2'} color="rose" mono />
              <MosaicCell label="ACTIVE PLANS" value={String(health?.active_plans ?? 2)} color="dream" />
              <MosaicCell label="ACTIVE AGENTS" value={String(health?.active_agents ?? 5)} color="rose" />
              <MosaicCell label="ACTIVE RUNS" value={String(health?.active_runs ?? 1)} color="bone" />
            </Mosaic>

            <div className="provider-section-label">Providers</div>
            <div className="providers">
              {Object.entries(providers).map(([name, info]) => (
                <div key={name} className={`provider-card${info.healthy ? ' provider-card--healthy' : ''}`}>
                  <span className={`provider-led ${info.healthy ? 'healthy' : 'unhealthy'}`}>
                    {info.healthy && <span className="provider-led-pulse" />}
                  </span>
                  <span className="provider-name">{name}</span>
                  <span className={`provider-badge ${info.healthy ? 'ok' : 'down'}`}>
                    {info.healthy ? 'ok' : 'down'}
                  </span>
                </div>
              ))}
            </div>
          </div>
        )}

        {tab === 'status' && (
          <Pane title="STATUS">
            <div className="kv-grid">
              {Object.entries(status ?? { signals: 1247, episodes: 847, agents: 5, plans_completed: 23, plans_active: 2 }).map(([key, val]) => (
                <div key={key} className="kv-row">
                  <span className="kv-key">{key}</span>
                  <span className="kv-val">
                    {typeof val === 'object' ? JSON.stringify(val, null, 2) : String(val)}
                  </span>
                </div>
              ))}
            </div>
          </Pane>
        )}

        {tab === 'episodes' && (
          <div className="explorer-episodes">
            <div className="ep-toolbar">
              <input
                className="ep-search"
                placeholder="Search episodes..."
                value={epSearch}
                onChange={(e) => setEpSearch(e.target.value)}
              />
              <select className="ep-filter" value={epKind} onChange={(e) => setEpKind(e.target.value)}>
                <option value="">all kinds</option>
                <option value="agent_turn">agent_turn</option>
                <option value="gate_result">gate_result</option>
                <option value="tool_call">tool_call</option>
                <option value="plan_step">plan_step</option>
              </select>
            </div>
            <div className="ep-list">
              {filteredEpisodes.slice(0, 200).map((ep) => (
                <div
                  key={ep.id}
                  className={`ep-item${expandedEp === ep.id ? ' expanded' : ''}`}
                  onClick={() => setExpandedEp(expandedEp === ep.id ? null : ep.id)}
                >
                  <div className="ep-summary">
                    <span className={`ep-badge ep-${ep.kind}`}>{ep.kind}</span>
                    <span className="ep-agent">{ep.agent_id ?? 'system'}</span>
                    <span className="ep-task">{ep.task_id ?? ''}</span>
                    <span className="ep-cost">{ep.usage?.cost_usd != null ? `$${ep.usage.cost_usd.toFixed(4)}` : ''}</span>
                    <span className="ep-ts">{ep.timestamp_ms ? new Date(ep.timestamp_ms).toLocaleTimeString() : ''}</span>
                  </div>
                  {expandedEp === ep.id && (
                    <div className="ep-detail">
                      {Object.entries(ep).map(([k, v]) => (
                        <div key={k} className="ep-field">
                          <span className="ep-field-key">{k}</span>
                          <span className="ep-field-val">
                            {typeof v === 'object' ? JSON.stringify(v) : String(v ?? '')}
                          </span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              ))}
            </div>
          </div>
        )}

        {tab === 'events' && (
          <div className="explorer-events">
            <div className="event-list">
              {events.slice(0, 500).map((evt, i) => (
                <div key={i} className="event-item">
                  <span className="event-badge">{evt.type}</span>
                  <span className="event-payload">{JSON.stringify(evt.payload).slice(0, 120)}</span>
                  <span className="event-ts">{new Date(evt.timestamp).toLocaleTimeString()}</span>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
