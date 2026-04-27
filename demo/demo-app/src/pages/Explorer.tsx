import { useState, useEffect, useCallback } from 'react';
import { useApi } from '../hooks/useApi';
import StatCard from '../components/StatCard';
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

interface Episode {
  id: string;
  kind: string;
  agent?: string;
  task?: string;
  model?: string;
  role?: string;
  input_tokens?: number;
  output_tokens?: number;
  cost_usd?: number;
  hdc_fingerprint?: string;
  gate_verdicts?: Record<string, string>;
  timestamp: string;
  [key: string]: unknown;
}

interface StateEvent {
  type: string;
  payload: unknown;
  timestamp: string;
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
  const { get } = useApi();

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

  return (
    <div className="explorer-page">
      <div className="explorer-header">
        <span className="explorer-title">explorer</span>
        <div className="explorer-tabs">
          {TABS.map((t) => (
            <button key={t.id} className={`explorer-tab${tab === t.id ? ' active' : ''}`} onClick={() => setTab(t.id)}>
              {t.label}
            </button>
          ))}
        </div>
        <button className="btn-refresh" onClick={refresh}>↻ Refresh</button>
      </div>

      <div className="explorer-body">
        {tab === 'health' && (
          <div className="explorer-health">
            <div className="health-grid">
              <StatCard
                label="Status"
                value={health?.status ?? '—'}
                color={health?.status === 'ok' ? 'sage' : 'fail'}
              />
              <StatCard
                label="Uptime"
                value={health?.uptime_secs ? `${Math.floor(health.uptime_secs / 60)}m` : '—'}
                color="bone"
              />
              <StatCard label="Version" value={health?.version ?? '—'} color="rose" />
              <StatCard label="Active Plans" value={health?.active_plans ?? 0} color="bone" />
              <StatCard label="Active Agents" value={health?.active_agents ?? 0} color="rose" />
              <StatCard label="Active Runs" value={health?.active_runs ?? 0} color="sage" />
            </div>
            {health?.providers && (
              <div className="provider-grid">
                <h3>Providers</h3>
                <div className="providers">
                  {Object.entries(health.providers).map(([name, info]) => (
                    <div key={name} className={`provider-card ${info.healthy ? 'healthy' : 'unhealthy'}`}>
                      <span className="provider-name">{name}</span>
                      <span className={`provider-badge ${info.healthy ? 'ok' : 'down'}`}>
                        {info.healthy ? 'ok' : 'down'}
                      </span>
                      {info.latency_ms != null && (
                        <span className="provider-latency">{info.latency_ms}ms</span>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
        )}

        {tab === 'status' && (
          <div className="explorer-status">
            {status ? (
              <div className="kv-grid">
                {Object.entries(status).map(([key, val]) => (
                  <div key={key} className="kv-row">
                    <span className="kv-key">{key}</span>
                    <span className="kv-val">
                      {typeof val === 'object' ? JSON.stringify(val, null, 2) : String(val)}
                    </span>
                  </div>
                ))}
              </div>
            ) : (
              <div className="explorer-empty">No status data — is the server running?</div>
            )}
          </div>
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
              {filteredEpisodes.length === 0 ? (
                <div className="explorer-empty">No episodes found</div>
              ) : (
                filteredEpisodes.slice(0, 200).map((ep) => (
                  <div
                    key={ep.id}
                    className={`ep-item${expandedEp === ep.id ? ' expanded' : ''}`}
                    onClick={() => setExpandedEp(expandedEp === ep.id ? null : ep.id)}
                  >
                    <div className="ep-summary">
                      <span className={`ep-badge ep-${ep.kind}`}>{ep.kind}</span>
                      <span className="ep-agent">{ep.agent ?? '—'}</span>
                      <span className="ep-task">{ep.task ?? ''}</span>
                      <span className="ep-cost">{ep.cost_usd != null ? `$${ep.cost_usd.toFixed(4)}` : ''}</span>
                      <span className="ep-ts">{new Date(ep.timestamp).toLocaleTimeString()}</span>
                    </div>
                    {expandedEp === ep.id && (
                      <div className="ep-detail">
                        {Object.entries(ep).map(([k, v]) => (
                          <div key={k} className="ep-field">
                            <span className="ep-field-key">{k}</span>
                            <span className="ep-field-val">
                              {typeof v === 'object' ? JSON.stringify(v) : String(v ?? '—')}
                            </span>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                ))
              )}
            </div>
          </div>
        )}

        {tab === 'events' && (
          <div className="explorer-events">
            <div className="event-list">
              {events.length === 0 ? (
                <div className="explorer-empty">No events</div>
              ) : (
                events.slice(0, 500).map((evt, i) => (
                  <div key={i} className="event-item">
                    <span className="event-badge">{evt.type}</span>
                    <span className="event-payload">{JSON.stringify(evt.payload).slice(0, 120)}</span>
                    <span className="event-ts">{new Date(evt.timestamp).toLocaleTimeString()}</span>
                  </div>
                ))
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
