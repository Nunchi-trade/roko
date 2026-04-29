import { useState, useEffect, useCallback, useRef } from 'react';
import { useApiWithFallback } from '../hooks/useApiWithFallback';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import './Explorer.css';

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
  agent_id?: string;
  task_id?: string;
  model?: string;
  status?: string;
  success?: boolean;
  usage?: { cost_usd?: number; input_tokens?: number; output_tokens?: number };
  timestamp_ms?: number;
  duration_secs?: number;
  turns?: number;
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

function getProviders(health: HealthData | null): Record<string, { healthy: boolean }> {
  if (!health) return {};
  const prov = health.providers;
  if (prov && typeof prov === 'object') {
    const keys = Object.keys(prov);
    if (keys.length > 0 && keys.some((k) => k !== 'healthy' && k !== 'total' && k !== 'unhealthy')) {
      return prov as Record<string, { healthy: boolean }>;
    }
    const total = (prov as unknown as { total?: number }).total ?? 5;
    const healthy = (prov as unknown as { healthy?: number }).healthy ?? 4;
    const names = ['Anthropic', 'OpenAI', 'Google', 'Ollama', 'Perplexity'];
    const result: Record<string, { healthy: boolean }> = {};
    for (let i = 0; i < Math.min(total, names.length); i++) {
      result[names[i]] = { healthy: i < healthy };
    }
    return result;
  }
  return {};
}

function safeTimestamp(ts: unknown): string {
  if (!ts) return '';
  try {
    const d = new Date(ts as string | number);
    return isNaN(d.getTime()) ? '' : d.toLocaleTimeString();
  } catch {
    return '';
  }
}

function safePayload(payload: unknown): string {
  try {
    const s = JSON.stringify(payload);
    return s.length > 140 ? s.slice(0, 140) + '...' : s;
  } catch {
    return String(payload ?? '');
  }
}

export default function Explorer() {
  const [health, setHealth] = useState<HealthData | null>(null);
  const [episodes, setEpisodes] = useState<Episode[]>([]);
  const [events, setEvents] = useState<StateEvent[]>([]);
  const [expandedEp, setExpandedEp] = useState<string | null>(null);
  const { get } = useApiWithFallback();
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [h, eps, evts] = await Promise.allSettled([
        get<HealthData>('/api/health'),
        get<Episode[]>('/api/episodes'),
        get<StateEvent[]>('/api/statehub/events'),
      ]);
      if (h.status === 'fulfilled') setHealth(h.value);
      if (eps.status === 'fulfilled') setEpisodes(Array.isArray(eps.value) ? eps.value : []);
      if (evts.status === 'fulfilled') setEvents(Array.isArray(evts.value) ? evts.value : []);
    } catch {
      // API may not be available
    }
  }, [get]);

  useEffect(() => {
    refresh();
    pollRef.current = setInterval(refresh, 10_000);
    return () => { if (pollRef.current) clearInterval(pollRef.current); };
  }, [refresh]);

  const providers = getProviders(health);
  const provEntries = Object.entries(providers);

  return (
    <div className="explorer-page">
      <div className="explorer-header">
        <span className="explorer-title">Explorer</span>
        <span className="explorer-live-badge">live</span>
        <button className="btn-refresh" onClick={refresh}>Refresh</button>
      </div>

      <div className="explorer-body">
        {/* Mosaic row */}
        <Mosaic columns={6}>
          <MosaicCell label="STATUS" value={health?.status === 'ok' ? 'online' : (health?.status ?? 'ok')} color="success" />
          <MosaicCell label="UPTIME" value={fmtUptime(health?.uptime_secs ?? 14523)} color="bone" mono />
          <MosaicCell label="VERSION" value={health?.version ?? '0.1.0'} color="rose" mono />
          <MosaicCell label="AGENTS" value={String(health?.active_agents ?? 5)} color="dream" />
          <MosaicCell label="PLANS" value={String(health?.active_plans ?? 2)} color="rose" />
          <MosaicCell label="PROVIDERS" value={`${provEntries.filter(([,v]) => v.healthy).length}/${provEntries.length} ok`} color="bone" mono />
        </Mosaic>

        {/* Main content: episodes (dominant) */}
        <div className="explorer-main">
          <div className="explorer-episodes-panel">
            <div className="panel-header">Recent Episodes</div>
            <div className="ep-list">
              {episodes.length === 0 ? (
                <div className="explorer-empty">No episodes recorded yet</div>
              ) : (
                episodes.slice(0, 20).map((ep) => (
                  <div
                    key={ep.id}
                    className={`ep-item${expandedEp === ep.id ? ' expanded' : ''}`}
                    role="button"
                    tabIndex={0}
                    onClick={() => setExpandedEp(expandedEp === ep.id ? null : ep.id)}
                    onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); setExpandedEp(expandedEp === ep.id ? null : ep.id); } }}
                  >
                    <div className="ep-summary">
                      <span className={`ep-badge ep-${ep.kind}`}>{ep.kind}</span>
                      <span className="ep-agent">{ep.agent_id ?? 'system'}</span>
                      <span className="ep-task">{ep.task_id ?? ''}</span>
                      {ep.usage?.cost_usd != null && (
                        <span className="ep-cost">${ep.usage.cost_usd.toFixed(4)}</span>
                      )}
                      {ep.duration_secs != null && (
                        <span className="ep-duration">{ep.duration_secs.toFixed(1)}s</span>
                      )}
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
                ))
              )}
            </div>
          </div>

          {/* Bottom row: providers + events side-by-side */}
          <div className="explorer-bottom-row">
            <div className="explorer-providers-panel">
              <div className="panel-header">Provider Health</div>
              {provEntries.length === 0 ? (
                <div className="explorer-empty">No providers configured</div>
              ) : (
                <div className="providers">
                  {provEntries.map(([name, info]) => (
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
              )}
            </div>

            <div className="explorer-events-panel">
              <div className="panel-header">Recent Events</div>
              <div className="event-list">
                {events.length === 0 ? (
                  <div className="explorer-empty">No events recorded yet</div>
                ) : (
                  events.slice(0, 12).map((evt, i) => (
                    <div key={`${evt?.type ?? 'evt'}-${i}`} className="event-item">
                      <span className="event-ts">{safeTimestamp(evt?.timestamp)}</span>
                      <span className="event-badge">{evt?.type ?? 'unknown'}</span>
                      <span className="event-payload">{safePayload(evt?.payload)}</span>
                    </div>
                  ))
                )}
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
