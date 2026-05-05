import { useState, useEffect, useRef } from 'react';
import Pane from '../../components/Pane';
import { SERVE_URL, WS_BASE } from '../../lib/serve-url';

/* ── Types ─────────────────────────────────────────────────── */

interface IsfrStatus {
  keeper_running: boolean;
  sources_count: number;
  current_epoch: number | null;
  poll_interval_secs: number;
}

interface IsfrRate {
  composite_bps: number;
  lending_bps: number;
  structured_bps: number;
  funding_bps: number;
  staking_bps: number;
  confidence_bps: number;
  timestamp_ms: number;
}

interface IsfrSource {
  id: string;
  name: string;
  class: string;
  weight: number;
  health: 'healthy' | 'degraded' | 'down';
  last_rate_bps: number;
  last_poll_ms: number | null;
}

interface RelayMessage {
  id: number;
  topic: string;
  msg_type: string;
  payload: unknown;
  publisher_id: string | null;
  seq: number;
  timestamp_ms: number;
}

type TabId = 'overview' | 'sources' | 'relay' | 'chain';

/* ── Helpers ───────────────────────────────────────────────── */

const MAX_HISTORY = 120;
const MAX_RELAY_MESSAGES = 200;

function formatBps(bps: number): string {
  return `${(bps / 100).toFixed(2)}%`;
}

function formatConfidence(bps: number): string {
  return `${(bps / 100).toFixed(0)}%`;
}

async function fetchJson<T>(path: string): Promise<T> {
  const res = await fetch(`${SERVE_URL}${path}`);
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
  return res.json();
}

/* ── Main Component ────────────────────────────────────────── */

export default function IsfrPage() {
  const [tab, setTab] = useState<TabId>('overview');
  const [status, setStatus] = useState<IsfrStatus | null>(null);
  const [currentRate, setCurrentRate] = useState<IsfrRate | null>(null);
  const [sources, setSources] = useState<IsfrSource[]>([]);
  const [history, setHistory] = useState<IsfrRate[]>([]);
  const [relayMessages, setRelayMessages] = useState<RelayMessage[]>([]);
  const [relayConnected, setRelayConnected] = useState(false);

  // ─── Polling: status, current rate, sources, history ──────
  useEffect(() => {
    let cancelled = false;

    async function poll() {
      try {
        const [s, r, src, h] = await Promise.all([
          fetchJson<IsfrStatus>('/api/isfr/status'),
          fetchJson<IsfrRate>('/api/isfr/rate/current'),
          fetchJson<IsfrSource[]>('/api/isfr/sources'),
          fetchJson<IsfrRate[]>(`/api/isfr/rate/history?limit=${MAX_HISTORY}`),
        ]);
        if (cancelled) return;
        setStatus(s);
        setCurrentRate(r);
        setSources(src);
        setHistory(h);
      } catch {
        // Silently ignore — data just won't update
      }
    }

    poll();
    const id = setInterval(poll, 5_000);
    return () => { cancelled = true; clearInterval(id); };
  }, []);

  // ─── WebSocket for relay tab (lazy — only when active) ────
  useEffect(() => {
    if (tab !== 'relay') return;

    let ws: WebSocket | null = null;
    let msgId = 0;

    try {
      ws = new WebSocket(`${WS_BASE}/api/workflow/ws`);

      ws.onopen = () => setRelayConnected(true);
      ws.onclose = () => setRelayConnected(false);
      ws.onerror = () => setRelayConnected(false);

      ws.onmessage = (ev) => {
        try {
          const data = JSON.parse(ev.data);
          // Accept topic messages in either envelope form
          const msg = (data as { TopicMessage?: RelayMessage }).TopicMessage ?? (data as RelayMessage);
          if (msg.topic && msg.topic.startsWith('isfr')) {
            setRelayMessages((prev) => [
              { ...msg, id: ++msgId },
              ...prev.slice(0, MAX_RELAY_MESSAGES - 1),
            ]);
          }
        } catch {
          // Ignore non-JSON frames
        }
      };
    } catch {
      // WebSocket creation failed
    }

    return () => {
      ws?.close();
      setRelayConnected(false);
    };
  }, [tab]);

  // ─── Render ───────────────────────────────────────────────
  const TABS: { id: TabId; label: string }[] = [
    { id: 'overview', label: 'Overview' },
    { id: 'sources', label: 'Sources' },
    { id: 'relay', label: 'Relay' },
    { id: 'chain', label: 'Chain' },
  ];

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--sp-3)' }}>
      {/* Tab bar */}
      <div style={{ display: 'flex', gap: 4 }}>
        {TABS.map((t) => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            style={{
              padding: 'var(--sp-1) var(--sp-3)',
              borderRadius: 'var(--radius-md)',
              border: '1px solid transparent',
              fontSize: 'var(--text-sm)',
              fontFamily: 'var(--mono, var(--font-mono))',
              letterSpacing: '.06em',
              textTransform: 'uppercase',
              cursor: 'pointer',
              transition: 'all .2s ease',
              ...(tab === t.id
                ? {
                    color: 'var(--rose-bright)',
                    background: 'var(--rose-deep)',
                    borderColor: 'var(--rose-dim)',
                  }
                : {
                    color: 'var(--text-muted)',
                    background: 'transparent',
                  }),
            }}
          >
            {t.label}
          </button>
        ))}
      </div>

      {/* Tab content */}
      {tab === 'overview' && (
        <OverviewTab status={status} currentRate={currentRate} history={history} />
      )}
      {tab === 'sources' && <SourcesTab sources={sources} />}
      {tab === 'relay' && (
        <RelayTab connected={relayConnected} messages={relayMessages} />
      )}
      {tab === 'chain' && <ChainTab />}
    </div>
  );
}

/* ── Overview Tab ──────────────────────────────────────────── */

function OverviewTab({
  status,
  currentRate,
  history,
}: {
  status: IsfrStatus | null;
  currentRate: IsfrRate | null;
  history: IsfrRate[];
}) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--sp-3)' }}>
      {/* Status banner */}
      <Pane title="ISFR Keeper" icon="status"
        badge={
          <span style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 6,
            fontSize: 'var(--text-xs)',
            color: status?.keeper_running ? 'var(--success)' : 'var(--text-muted)',
          }}>
            <span style={{
              width: 8, height: 8, borderRadius: '50%',
              background: status?.keeper_running ? 'var(--success)' : 'var(--text-muted)',
            }} />
            {status?.keeper_running ? 'Running' : 'Stopped'}
          </span>
        }
      >
        <div style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(3, 1fr)',
          gap: 'var(--sp-3)',
          fontFamily: 'var(--mono, var(--font-mono))',
          fontSize: 'var(--text-sm)',
        }}>
          <div>
            <div style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>Sources</div>
            <div style={{ color: 'var(--text-primary)' }}>{status?.sources_count ?? '—'}</div>
          </div>
          <div>
            <div style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>Epoch</div>
            <div style={{ color: 'var(--text-primary)' }}>{status?.current_epoch ?? '—'}</div>
          </div>
          <div>
            <div style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>Poll Interval</div>
            <div style={{ color: 'var(--text-primary)' }}>{status?.poll_interval_secs ?? '—'}s</div>
          </div>
        </div>
      </Pane>

      {/* Current rate */}
      {currentRate && (
        <Pane title="Current Rate" icon="activity">
          <div style={{ marginBottom: 'var(--sp-2)' }}>
            <span style={{
              fontFamily: 'var(--mono, var(--font-mono))',
              fontSize: '2rem',
              fontWeight: 700,
              color: 'var(--rose-bright)',
            }}>
              {formatBps(currentRate.composite_bps)}
            </span>
            <span style={{
              marginLeft: 'var(--sp-2)',
              fontSize: 'var(--text-xs)',
              color: 'var(--text-muted)',
            }}>
              APR &middot; Confidence {formatConfidence(currentRate.confidence_bps)}
            </span>
          </div>
          <div style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(4, 1fr)',
            gap: 'var(--sp-2)',
            fontFamily: 'var(--mono, var(--font-mono))',
            fontSize: 'var(--text-sm)',
          }}>
            <div>
              <div style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>Lending</div>
              <div>{formatBps(currentRate.lending_bps)}</div>
            </div>
            <div>
              <div style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>Structured</div>
              <div>{formatBps(currentRate.structured_bps)}</div>
            </div>
            <div>
              <div style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>Funding</div>
              <div>{formatBps(currentRate.funding_bps)}</div>
            </div>
            <div>
              <div style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>Staking</div>
              <div>{formatBps(currentRate.staking_bps)}</div>
            </div>
          </div>
        </Pane>
      )}

      {/* History table */}
      {history.length > 0 && (
        <Pane title="Recent Rates" icon="clock"
          badge={<span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>Last {Math.min(history.length, 10)}</span>}
        >
          <div style={{ overflowX: 'auto' }}>
            <table style={{ width: '100%', fontFamily: 'var(--mono, var(--font-mono))', fontSize: 'var(--text-xs)', borderCollapse: 'collapse' }}>
              <thead>
                <tr style={{ color: 'var(--text-muted)', borderBottom: '1px solid var(--glass-2-border)' }}>
                  <th style={{ textAlign: 'left', padding: 'var(--sp-1) var(--sp-2)' }}>Time</th>
                  <th style={{ textAlign: 'right', padding: 'var(--sp-1) var(--sp-2)' }}>Composite</th>
                  <th style={{ textAlign: 'right', padding: 'var(--sp-1) var(--sp-2)' }}>Lending</th>
                  <th style={{ textAlign: 'right', padding: 'var(--sp-1) var(--sp-2)' }}>Struct</th>
                  <th style={{ textAlign: 'right', padding: 'var(--sp-1) var(--sp-2)' }}>Funding</th>
                  <th style={{ textAlign: 'right', padding: 'var(--sp-1) var(--sp-2)' }}>Staking</th>
                  <th style={{ textAlign: 'right', padding: 'var(--sp-1) var(--sp-2)' }}>Conf</th>
                </tr>
              </thead>
              <tbody>
                {history.slice(-10).reverse().map((r, i) => (
                  <tr key={i} style={{ borderBottom: '1px solid var(--glass-2-border)' }}>
                    <td style={{ textAlign: 'left', padding: 'var(--sp-1) var(--sp-2)', color: 'var(--text-muted)' }}>
                      {new Date(r.timestamp_ms).toLocaleTimeString()}
                    </td>
                    <td style={{ textAlign: 'right', padding: 'var(--sp-1) var(--sp-2)', color: 'var(--text-primary)' }}>
                      {formatBps(r.composite_bps)}
                    </td>
                    <td style={{ textAlign: 'right', padding: 'var(--sp-1) var(--sp-2)' }}>{formatBps(r.lending_bps)}</td>
                    <td style={{ textAlign: 'right', padding: 'var(--sp-1) var(--sp-2)' }}>{formatBps(r.structured_bps)}</td>
                    <td style={{ textAlign: 'right', padding: 'var(--sp-1) var(--sp-2)' }}>{formatBps(r.funding_bps)}</td>
                    <td style={{ textAlign: 'right', padding: 'var(--sp-1) var(--sp-2)' }}>{formatBps(r.staking_bps)}</td>
                    <td style={{ textAlign: 'right', padding: 'var(--sp-1) var(--sp-2)' }}>{formatConfidence(r.confidence_bps)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Pane>
      )}
    </div>
  );
}

/* ── Sources Tab ───────────────────────────────────────────── */

function SourcesTab({ sources }: { sources: IsfrSource[] }) {
  if (sources.length === 0) {
    return (
      <Pane title="Sources" icon="database">
        <p style={{ color: 'var(--text-muted)', fontSize: 'var(--text-sm)' }}>
          No sources configured. Start the ISFR keeper to populate sources.
        </p>
      </Pane>
    );
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--sp-2)' }}>
      {sources.map((src) => (
        <Pane
          key={src.id}
          title={src.name}
          icon="activity"
          badge={
            <span style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 6,
              fontSize: 'var(--text-xs)',
            }}>
              <span style={{
                width: 6, height: 6, borderRadius: '50%',
                background:
                  src.health === 'healthy' ? 'var(--success)'
                  : src.health === 'degraded' ? 'var(--warning)'
                  : 'var(--error)',
              }} />
              {src.health}
            </span>
          }
        >
          <div style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(4, 1fr)',
            gap: 'var(--sp-2)',
            fontFamily: 'var(--mono, var(--font-mono))',
            fontSize: 'var(--text-sm)',
          }}>
            <div>
              <div style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>Class</div>
              <div style={{ color: 'var(--text-primary)' }}>{src.class}</div>
            </div>
            <div>
              <div style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>Weight</div>
              <div style={{ color: 'var(--text-primary)' }}>{(src.weight * 100).toFixed(0)}%</div>
            </div>
            <div>
              <div style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>Last Rate</div>
              <div style={{ color: 'var(--text-primary)' }}>{formatBps(src.last_rate_bps)}</div>
            </div>
            <div>
              <div style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>Last Poll</div>
              <div style={{ color: 'var(--text-primary)' }}>
                {src.last_poll_ms ? new Date(src.last_poll_ms).toLocaleTimeString() : '—'}
              </div>
            </div>
          </div>
        </Pane>
      ))}
    </div>
  );
}

/* ── Relay Tab ─────────────────────────────────────────────── */

function RelayTab({
  connected,
  messages,
}: {
  connected: boolean;
  messages: RelayMessage[];
}) {
  const scrollRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to top on new messages (newest first)
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = 0;
    }
  }, [messages.length]);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--sp-2)' }}>
      {/* Connection status */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <span style={{
          width: 8, height: 8, borderRadius: '50%',
          background: connected ? 'var(--success)' : 'var(--error)',
        }} />
        <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>
          Relay {connected ? 'connected' : 'disconnected'}
          {' '}&middot; Showing ISFR topic messages
        </span>
      </div>

      {/* Message log */}
      <div
        ref={scrollRef}
        style={{
          maxHeight: '65vh',
          overflowY: 'auto',
          display: 'flex',
          flexDirection: 'column',
          gap: 4,
        }}
      >
        {messages.length === 0 && (
          <p style={{ color: 'var(--text-muted)', fontSize: 'var(--text-sm)', fontStyle: 'italic' }}>
            Waiting for ISFR topic messages...
          </p>
        )}
        {messages.map((msg) => (
          <div
            key={msg.id}
            style={{
              padding: 'var(--sp-1) var(--sp-2)',
              borderRadius: 'var(--radius-sm)',
              background: 'var(--bg-deeper)',
              border: '1px solid var(--glass-2-border)',
              fontFamily: 'var(--mono, var(--font-mono))',
              fontSize: 'var(--text-xs)',
            }}
          >
            <div style={{ display: 'flex', justifyContent: 'space-between', color: 'var(--text-muted)' }}>
              <span style={{ color: 'var(--rose-bright)' }}>{msg.topic}</span>
              <span>seq:{msg.seq} &middot; {new Date(msg.timestamp_ms).toLocaleTimeString()}</span>
            </div>
            <div style={{ color: 'var(--text-secondary)', marginTop: 2 }}>
              {msg.msg_type}: {JSON.stringify(msg.payload).slice(0, 200)}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

/* ── Chain Tab ─────────────────────────────────────────────── */

function ChainTab() {
  return (
    <Pane title="Chain Events" icon="hash">
      <div style={{ color: 'var(--text-muted)', fontSize: 'var(--text-sm)' }}>
        <p>Chain event stream — requires the A4 chain watcher to be running.</p>
        <p style={{ marginTop: 'var(--sp-2)' }}>
          When active, this tab will show real-time block events and contract
          interactions (RateSubmitted, BountyClaimed, EpochFinalized).
        </p>
        <p style={{ marginTop: 'var(--sp-2)', fontStyle: 'italic' }}>
          Placeholder until chain watcher integration is complete.
        </p>
      </div>
    </Pane>
  );
}
