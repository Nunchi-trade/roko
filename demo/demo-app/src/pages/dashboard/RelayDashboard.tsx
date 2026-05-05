/**
 * RelayDashboard — real-time view of the agent-relay.
 *
 * Layout: event stream as the hero (left), sidebar with agents/workspaces/topics/feeds (right).
 * REST-fetches initial state, streams updates via WS at /relay/events/ws.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { useDataHub } from '../../app/DataHub';
import { WS_BASE } from '../../lib/serve-url';
import type { RelayEvent } from '../../lib/relay-api';
import type { RelayEventLogEntry } from '../../app/DataHub';
import './RelayDashboard.css';

// ── WS hook ────────────────────────────────────────────────

type WsConnStatus = 'disconnected' | 'connecting' | 'connected' | 'reconnecting';

function useRelayEvents(onEvent: (ev: RelayEvent) => void) {
  const [status, setStatus] = useState<WsConnStatus>('disconnected');
  const onEventRef = useRef(onEvent);
  onEventRef.current = onEvent;

  useEffect(() => {
    let ws: WebSocket | null = null;
    let retryCount = 0;
    let retryTimer: ReturnType<typeof setTimeout> | null = null;
    let destroyed = false;

    function connect() {
      if (destroyed) return;
      setStatus(retryCount === 0 ? 'connecting' : 'reconnecting');

      // Use the same origin for WS so Vite's proxy handles the upgrade.
      // WS_BASE resolves to ws://localhost:5173 in dev → Vite proxies to :6677.
      // In prod/Docker, WS_BASE resolves to the public origin.
      const url = `${WS_BASE}/relay/events/ws`;

      try {
        ws = new WebSocket(url);
      } catch {
        // WebSocket constructor can throw for invalid URLs
        scheduleRetry();
        return;
      }

      ws.onopen = () => {
        if (destroyed) return;
        retryCount = 0;
        setStatus('connected');
      };

      ws.onmessage = (e: MessageEvent) => {
        if (destroyed) return;
        try {
          const event = JSON.parse(e.data as string) as RelayEvent;
          onEventRef.current(event);
        } catch {
          // skip unparseable frames
        }
      };

      ws.onerror = () => {
        // handled in onclose
      };

      ws.onclose = () => {
        if (destroyed) {
          setStatus('disconnected');
          return;
        }
        scheduleRetry();
      };
    }

    function scheduleRetry() {
      retryCount += 1;
      // Cap at 3 retries to avoid spamming console when relay isn't running.
      if (retryCount > 3) {
        setStatus('disconnected');
        return;
      }
      setStatus('reconnecting');
      const delay = Math.min(2000 * 2 ** (retryCount - 1), 10_000);
      retryTimer = setTimeout(connect, delay);
    }

    connect();

    return () => {
      destroyed = true;
      if (retryTimer !== null) clearTimeout(retryTimer);
      if (ws) ws.close();
    };
  }, []);

  return status;
}

// ── Helpers ────────────────────────────────────────────────

function formatTime(ts: number): string {
  return new Date(ts).toLocaleTimeString('en-GB', { hour12: false });
}

function timeAgo(ms: number): string {
  const diff = Date.now() - ms;
  if (diff < 1000) return 'now';
  if (diff < 60_000) return `${Math.floor(diff / 1000)}s`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m`;
  return `${Math.floor(diff / 3_600_000)}h`;
}

/** Color token for event type — gives visual scanning. */
function eventColor(type: string): string {
  if (type.includes('connected') && !type.includes('dis')) return 'var(--green, #98c379)';
  if (type.includes('disconnected')) return 'var(--red, #e06c75)';
  if (type.includes('heartbeat')) return 'var(--text-tertiary)';
  if (type.includes('feed')) return 'var(--cyan, #56b6c2)';
  if (type.includes('error')) return 'var(--red, #e06c75)';
  if (type.includes('message')) return 'var(--yellow, #e5c07b)';
  if (type.includes('card')) return 'var(--magenta, #b5838d)';
  return 'var(--text-secondary)';
}

/** Short label for event type. */
function eventLabel(type: string): string {
  return type.replace(/_/g, ' ').replace(/^(agent|workspace|feed)\s/, '');
}

// ── Subcomponents ──────────────────────────────────────────

function StatusDot({ variant }: { variant: 'ok' | 'warn' | 'err' }) {
  const cls = variant === 'ok' ? 'rdot rdot--ok' : variant === 'warn' ? 'rdot rdot--warn' : 'rdot rdot--err';
  return <span className={cls} />;
}

function EventRow({ entry }: { entry: RelayEventLogEntry }) {
  return (
    <div className="rlog-row">
      <span className="rlog-row__time">{formatTime(entry.ts)}</span>
      <span className="rlog-row__badge" style={{ color: eventColor(entry.type) }}>
        {eventLabel(entry.type)}
      </span>
      <span className="rlog-row__msg">{entry.message}</span>
    </div>
  );
}

// ── Component ──────────────────────────────────────────────

export default function RelayDashboard() {
  const agents = useDataHub((s) => s.relayDashAgents);
  const workspaces = useDataHub((s) => s.relayDashWorkspaces);
  const feeds = useDataHub((s) => s.relayDashFeeds);
  const topics = useDataHub((s) => s.relayDashTopics);
  const eventLog = useDataHub((s) => s.relayDashEventLog);
  const fetchRelayDashboard = useDataHub((s) => s.fetchRelayDashboard);
  const handleRelayEvent = useDataHub((s) => s.handleRelayEvent);

  // Tick every 5s so "time ago" labels stay fresh.
  const [, setTick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), 5_000);
    return () => clearInterval(id);
  }, []);

  useEffect(() => {
    fetchRelayDashboard();
  }, [fetchRelayDashboard]);

  const wsStatus = useRelayEvents(
    useCallback(
      (ev: RelayEvent) => handleRelayEvent(ev),
      [handleRelayEvent],
    ),
  );

  const allFeeds = feeds.flatMap((g) => g.feeds);
  const totalSubs = topics.reduce((sum, t) => sum + (typeof t.subscribers === 'number' ? t.subscribers : t.subscribers.length), 0);

  return (
    <div className="relay">
      {/* ── Stat bar ─────────────────────────────────────── */}
      <div className="relay__stats">
        <div className="relay__stat-cell">
          <span className="relay__stat-value">{agents.length}</span>
          <span className="relay__stat-label">agents</span>
        </div>
        <div className="relay__stat-cell">
          <span className="relay__stat-value">{workspaces.length}</span>
          <span className="relay__stat-label">workspaces</span>
        </div>
        <div className="relay__stat-cell">
          <span className="relay__stat-value">{topics.length}</span>
          <span className="relay__stat-label">topics</span>
        </div>
        <div className="relay__stat-cell">
          <span className="relay__stat-value">{totalSubs}</span>
          <span className="relay__stat-label">subscriptions</span>
        </div>
        <div className="relay__stat-cell">
          <span className="relay__stat-value">{allFeeds.length}</span>
          <span className="relay__stat-label">feeds</span>
        </div>
        <div className="relay__stat-cell relay__stat-cell--ws">
          <StatusDot variant={wsStatus === 'connected' ? 'ok' : wsStatus === 'disconnected' ? 'err' : 'warn'} />
          <span className="relay__stat-label">
            {wsStatus === 'connected' ? 'live' : wsStatus}
          </span>
        </div>
      </div>

      {/* ── Main body ────────────────────────────────────── */}
      <div className="relay__body">

        {/* ── Left: event stream (the hero) ──────────────── */}
        <div className="relay__stream">
          <div className="relay__section-hdr">
            <span className="relay__section-title">EVENT STREAM</span>
            <span className="relay__section-count">{eventLog.length}</span>
          </div>
          <div className="relay__stream-scroll">
            {eventLog.length === 0 ? (
              <div className="relay__empty">
                Waiting for relay events...
              </div>
            ) : (
              eventLog.slice(0, 100).map((entry, i) => (
                <EventRow key={`${entry.ts}-${i}`} entry={entry} />
              ))
            )}
          </div>
        </div>

        {/* ── Right: sidebar panels ──────────────────────── */}
        <div className="relay__sidebar">

          {/* Agents */}
          <div className="relay__panel">
            <div className="relay__section-hdr">
              <span className="relay__section-title">AGENTS</span>
              <span className="relay__section-count">{agents.length}</span>
            </div>
            <div className="relay__panel-scroll">
              {agents.length === 0 && (
                <div className="relay__empty">No agents connected</div>
              )}
              {agents.map((agent) => (
                <div key={agent.agent_id} className="ragent">
                  <div className="ragent__row">
                    <StatusDot variant="ok" />
                    <span className="ragent__name">{agent.name ?? agent.agent_id}</span>
                    <span className="ragent__time">{timeAgo(agent.connected_at_ms)}</span>
                  </div>
                  {agent.capabilities.length > 0 && (
                    <div className="ragent__caps">
                      {agent.capabilities.map((cap) => (
                        <span key={cap} className="ragent__cap-badge">{cap}</span>
                      ))}
                    </div>
                  )}
                  {agent.rest_endpoint && (
                    <div className="ragent__endpoint">{agent.rest_endpoint}</div>
                  )}
                </div>
              ))}
            </div>
          </div>

          {/* Workspaces */}
          <div className="relay__panel">
            <div className="relay__section-hdr">
              <span className="relay__section-title">WORKSPACES</span>
              <span className="relay__section-count">{workspaces.length}</span>
            </div>
            <div className="relay__panel-scroll">
              {workspaces.length === 0 && (
                <div className="relay__empty">No workspaces connected</div>
              )}
              {workspaces.map((ws) => {
                const hbAge = Date.now() - ws.last_heartbeat_ms;
                const healthy = hbAge < 30_000;
                return (
                  <div key={ws.workspace_id} className="rws">
                    <div className="rws__row">
                      <StatusDot variant={healthy ? 'ok' : 'err'} />
                      <span className="rws__name">{ws.name ?? ws.workspace_id}</span>
                    </div>
                    <div className="rws__url">{ws.url}</div>
                    <div className="rws__meta">
                      <span>{ws.agents_count} agents</span>
                      <span>hb {timeAgo(ws.last_heartbeat_ms)}</span>
                      {ws.version && <span>v{ws.version}</span>}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>

          {/* Topics */}
          <div className="relay__panel">
            <div className="relay__section-hdr">
              <span className="relay__section-title">TOPICS</span>
              <span className="relay__section-count">{topics.length}</span>
            </div>
            <div className="relay__panel-scroll">
              {topics.length === 0 && (
                <div className="relay__empty">No active topics</div>
              )}
              {topics.map((t) => (
                <div key={t.topic} className="rtopic">
                  <span className="rtopic__name">{t.topic}</span>
                  <span className="rtopic__subs">{typeof t.subscribers === 'number' ? t.subscribers : t.subscribers.length}</span>
                </div>
              ))}
            </div>
          </div>

          {/* Feeds */}
          {allFeeds.length > 0 && (
            <div className="relay__panel">
              <div className="relay__section-hdr">
                <span className="relay__section-title">FEEDS</span>
                <span className="relay__section-count">{allFeeds.length}</span>
              </div>
              <div className="relay__panel-scroll">
                {allFeeds.map((f) => (
                  <div key={f.feed_id} className="rfeed">
                    <span className="rfeed__name">{f.name}</span>
                    <span className="rfeed__topic">{f.topic}</span>
                    <span className="rfeed__kind">{f.kind || 'raw'}</span>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
