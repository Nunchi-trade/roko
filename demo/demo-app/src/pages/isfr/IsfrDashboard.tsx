/**
 * ISFR Dashboard — data-focused panel layout.
 *
 * Top: headline rate + confidence gauge + keeper status + epoch info.
 * Middle: field breakdown (left) + source table (right).
 * Bottom: history chart + event log.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useDataHub } from '../../app/DataHub';
import type {
  IsfrSource, IsfrRate, IsfrEventEntry,
  RelayAgentEntry, RelayFeed, FeedLogEntry,
  ChainBlockEntry, ChainTxEntry, ChainEventEntry,
} from '../../app/DataHub';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
import { useCountUp, fmtCount } from '../../hooks/useCountUp';
import { useCanvasSetup } from '../../hooks/useCanvasSetup';
import { formatBps, formatPercent } from '../../lib/isfr-api';
import Oscilloscope from '../../components/canvas/Oscilloscope';
import './IsfrDashboard.css';

// ── Helpers ────────────────────────────────────────────────

const FIELD_META: { key: keyof IsfrRate; field: string; label: string; color: string }[] = [
  { key: 'lendingBps',    field: 'lending',    label: 'Lending',    color: '#98c379' },
  { key: 'structuredBps', field: 'structured', label: 'Structured', color: '#61afef' },
  { key: 'stakingBps',    field: 'staking',    label: 'Staking',    color: '#e5c07b' },
  { key: 'fundingBps',    field: 'funding',    label: 'Funding',    color: '#c9a0a8' },
];

const HEALTH_CLS: Record<string, string> = {
  live: 'isfr-dot isfr-dot--live',
  stale: 'isfr-dot isfr-dot--stale',
  offline: 'isfr-dot isfr-dot--offline',
};

const EVT_CLR: Record<string, string> = {
  rate: '#98c379', source: '#61afef', keeper: '#e5c07b',
};

function fmtTime(ts: number): string {
  return new Date(ts).toLocaleTimeString('en-GB', { hour12: false });
}

// ── Main component ─────────────────────────────────────────

export default function IsfrDashboard() {
  const currentRate = useDataHub((s) => s.isfrCurrentRate);
  const sources = useDataHub((s) => s.isfrSources);
  const history = useDataHub((s) => s.isfrHistory);
  const keeper = useDataHub((s) => s.isfrKeeperStatus);
  const fieldHistory = useDataHub((s) => s.isfrFieldHistory);
  const eventLog = useDataHub((s) => s.isfrEventLog);
  const sourceHistory = useDataHub((s) => s.isfrSourceHistory);
  const fetchIsfrCurrent = useDataHub((s) => s.fetchIsfrCurrent);
  const fetchIsfrSources = useDataHub((s) => s.fetchIsfrSources);
  const fetchIsfrHistory = useDataHub((s) => s.fetchIsfrHistory);
  const fetchIsfrStatus = useDataHub((s) => s.fetchIsfrStatus);

  // Feed agent + chain state
  const relayAgents = useDataHub((s) => s.relayAgents);
  const relayFeeds = useDataHub((s) => s.relayFeeds);
  const feedLog = useDataHub((s) => s.feedLog);
  const chainBlocks = useDataHub((s) => s.chainBlocks);
  const chainTxs = useDataHub((s) => s.chainTxs);
  const chainEvents = useDataHub((s) => s.chainEvents);
  const chainLatestBlock = useDataHub((s) => s.chainLatestBlock);
  const chainWatcherRunning = useDataHub((s) => s.chainWatcherRunning);
  const chainGasHistory = useDataHub((s) => s.chainGasHistory);
  const fetchFeedCatalog = useDataHub((s) => s.fetchFeedCatalog);
  const fetchChainBlocks = useDataHub((s) => s.fetchChainBlocks);
  const fetchChainTxs = useDataHub((s) => s.fetchChainTxs);
  const fetchChainEvents = useDataHub((s) => s.fetchChainEvents);
  const fetchChainStatus = useDataHub((s) => s.fetchChainStatus);

  // Initial fetch
  useEffect(() => {
    fetchIsfrStatus();
    fetchIsfrCurrent();
    fetchIsfrSources();
    fetchIsfrHistory();
    fetchFeedCatalog();
    fetchChainBlocks();
    fetchChainTxs();
    fetchChainEvents();
    fetchChainStatus();
  }, [fetchIsfrStatus, fetchIsfrCurrent, fetchIsfrSources, fetchIsfrHistory,
      fetchFeedCatalog, fetchChainBlocks, fetchChainTxs, fetchChainEvents, fetchChainStatus]);

  // SSE subscriptions — ISFR
  const fetchAll = useCallback(() => {
    fetchIsfrSources();
  }, [fetchIsfrSources]);

  const debouncedRefetch = useDebouncedRefetch(fetchAll, 2000);
  useContextEventSubscription(
    ['isfr_rate_computed', 'isfr_source_health_changed', 'isfr_keeper_state_changed'],
    useCallback(() => debouncedRefetch(), [debouncedRefetch]),
  );

  // Chain section collapsible
  const [chainOpen, setChainOpen] = useState(true);

  // SSE subscriptions — Chain (activity tracking only; DataHub handles updates)
  const [chainSseActive, setChainSseActive] = useState(false);
  const chainSseTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useContextEventSubscription(
    ['chain_block', 'chain_tx', 'chain_contract_event'],
    useCallback(() => {
      setChainSseActive(true);
      clearTimeout(chainSseTimer.current);
      chainSseTimer.current = setTimeout(() => setChainSseActive(false), 10_000);
    }, []),
  );
  useEffect(() => () => clearTimeout(chainSseTimer.current), []);

  // SSE subscriptions — Feeds
  const debouncedFeedRefetch = useDebouncedRefetch(fetchFeedCatalog, 3000);
  useContextEventSubscription(
    ['feed_tick', 'feed_agent_online', 'feed_agent_offline'],
    useCallback(() => debouncedFeedRefetch(), [debouncedFeedRefetch]),
  );

  const compositeBps = currentRate?.compositeBps ?? 0;
  const confidencePct = (currentRate?.confidenceBps ?? 0) / 100;
  const sourceCount = currentRate?.sourceCount ?? 0;
  const liveCount = sources.filter((s) => s.health === 'live').length;

  const delta = useMemo(() => {
    if (history.length < 2) return 0;
    return history[history.length - 1].compositeBps - history[history.length - 2].compositeBps;
  }, [history]);

  return (
    <div className="isfr">
      {/* ── Row 1: Headline stats ──────────────────────── */}
      <div className="isfr__top">
        <HeadlineRate bps={compositeBps} delta={delta} loading={currentRate === null} />
        <ConfidenceGauge pct={confidencePct} />
        <StatCell label="Keeper" value={keeper === 'running' ? 'Active' : 'Idle'}
          dot={keeper === 'running' ? 'live' : 'offline'} />
        <StatCell label="Sources" value={`${liveCount}/${sourceCount}`}
          dot={liveCount === sourceCount ? 'live' : liveCount > 0 ? 'stale' : 'offline'} />
        <StatCell label="Epoch" value={String(history.length)} />
        <StatCell label="Samples" value={String(history.length)} />
      </div>

      {/* ── Row 2: Field breakdown + Source table ──────── */}
      <div className="isfr__mid">
        <div className="isfr__panel isfr__fields">
          <div className="isfr__panel-hdr">RATE BREAKDOWN</div>
          {FIELD_META.map((f) => {
            const bps = currentRate?.[f.key] ?? 0;
            const spark = fieldHistory[f.field as keyof typeof fieldHistory] ?? [];
            const hasSourcesForField = f.field === 'funding'
              ? sources.some((s) => s.class === 'funding')
              : true;
            return (
              <FieldRow key={f.key} label={f.label} bps={bps as number}
                color={f.color} sparkline={spark} noSources={!hasSourcesForField} />
            );
          })}
          <div className="isfr__field-total">
            <span>Composite</span>
            <span className="isfr__field-total-val">{formatBps(compositeBps)}</span>
          </div>
        </div>

        <div className="isfr__panel isfr__sources">
          <div className="isfr__panel-hdr">
            SOURCES
            <span className="isfr__badge">{sources.length}</span>
          </div>
          <SourceTable sources={sources} sourceHistory={sourceHistory} />
        </div>
      </div>

      {/* ── Row 3: History chart + Event log ───────────── */}
      <div className="isfr__bot">
        <div className="isfr__panel isfr__history">
          <div className="isfr__panel-hdr">COMPOSITE RATE HISTORY</div>
          <HistoryChart history={history} />
        </div>

        <div className="isfr__panel isfr__events">
          <div className="isfr__panel-hdr">
            EVENT LOG
            <span className="isfr__badge">{eventLog.length}</span>
          </div>
          <EventLog events={eventLog} />
        </div>
      </div>

      {/* ── Row 4: Feed Agents + Feed Log ────────────────── */}
      <div className="isfr__row4">
        <div className="isfr__panel isfr__agents-panel">
          <div className="isfr__panel-hdr">
            FEED AGENTS
            <span className="isfr__badge">{relayAgents.length}</span>
          </div>
          <AgentList agents={relayAgents} feeds={relayFeeds} />
        </div>

        <div className="isfr__panel isfr__feedlog-panel">
          <div className="isfr__panel-hdr">
            FEED LOG
            <span className="isfr__badge">{feedLog.length}</span>
          </div>
          <FeedLog entries={feedLog} />
        </div>
      </div>

      {/* ── Row 5: Chain Explorer (collapsible) ────────── */}
      <div className="isfr__chain-section">
        <button className="isfr__chain-toggle" onClick={() => setChainOpen((o) => !o)}>
          <span className={`isfr__chain-chevron${chainOpen ? ' isfr__chain-chevron--open' : ''}`}>&#x25B6;</span>
          CHAIN EXPLORER
        </button>
      </div>
      {chainOpen && <div className="isfr__row5">
        <div className="isfr__panel isfr__chain-blocks">
          <div className="isfr__panel-hdr">
            <span>BLOCK FEED</span>
            <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
              {chainSseActive && (
                <span className="isfr__sse-live"><span className="isfr__sse-dot" />LIVE</span>
              )}
              <span className={`isfr-dot ${chainWatcherRunning ? 'isfr-dot--live' : 'isfr-dot--offline'}`} />
              #{chainLatestBlock?.number ?? '---'}
            </span>
          </div>
          <ChainBlockFeed blocks={chainBlocks} />
        </div>

        <div className="isfr__panel isfr__chain-gas">
          <div className="isfr__panel-hdr">
            <span>GAS UTILIZATION</span>
            <span className="isfr__badge">{chainGasHistory.length} blocks</span>
          </div>
          <div className="isfr__gas-chart">
            <Oscilloscope data={chainGasHistory} height={140} />
          </div>
        </div>

        <div className="isfr__panel isfr__chain-txs">
          <div className="isfr__panel-hdr">
            <span>TRANSACTIONS</span>
            <span className="isfr__badge">{chainTxs.length}</span>
          </div>
          <ChainTxFeed txs={chainTxs} />
        </div>

        <div className="isfr__panel isfr__chain-events">
          <div className="isfr__panel-hdr">
            <span>CONTRACT EVENTS</span>
            <span className="isfr__badge">{chainEvents.length}</span>
          </div>
          <ChainEventFeed events={chainEvents} />
        </div>
      </div>}
    </div>
  );
}

// ── Subcomponents ──────────────────────────────────────────

function HeadlineRate({ bps, delta, loading }: { bps: number; delta: number; loading?: boolean }) {
  const anim = useCountUp(bps, 900);
  if (loading) {
    return (
      <div className="isfr__headline">
        <div className="isfr__headline-main">
          <span className="isfr__headline-val isfr__headline-val--loading">&mdash;</span>
          <span className="isfr__headline-unit">bps</span>
        </div>
      </div>
    );
  }
  return (
    <div className="isfr__headline">
      <div className="isfr__headline-main">
        <span className="isfr__headline-val">{fmtCount(Math.round(anim), 0)}</span>
        <span className="isfr__headline-unit">bps</span>
      </div>
      {delta !== 0 && (
        <span className={`isfr__headline-delta ${delta > 0 ? 'isfr__headline-delta--up' : 'isfr__headline-delta--down'}`}>
          {delta > 0 ? '\u25B2' : '\u25BC'} {Math.abs(delta)} bps
        </span>
      )}
    </div>
  );
}

function ConfidenceGauge({ pct }: { pct: number }) {
  const anim = useCountUp(pct, 800);
  const capped = Math.min(anim, 100);
  const color = capped >= 70 ? '#98c379' : capped >= 40 ? '#e5c07b' : '#e06c75';
  return (
    <div className="isfr__gauge">
      <div className="isfr__gauge-track">
        <div className="isfr__gauge-fill" style={{ width: `${capped}%`, background: color }} />
      </div>
      <div className="isfr__gauge-label">
        <span style={{ color, fontWeight: 600 }}>{Math.round(anim)}%</span>
        <span>confidence</span>
      </div>
    </div>
  );
}

function StatCell({ label, value, dot }: { label: string; value: string; dot?: string }) {
  return (
    <div className="isfr__stat">
      <span className="isfr__stat-label">{label}</span>
      <div className="isfr__stat-row">
        {dot && <span className={HEALTH_CLS[dot] ?? 'isfr-dot'} />}
        <span className="isfr__stat-value">{value}</span>
      </div>
    </div>
  );
}

function FieldRow({ label, bps, color, sparkline, noSources }: {
  label: string; bps: number; color: string; sparkline: number[]; noSources?: boolean;
}) {
  const anim = useCountUp(bps, 800);
  const weight = label === 'Lending' ? 60 : label === 'Structured' ? 25 : label === 'Funding' ? 10 : 5;
  return (
    <div className="isfr__field-row">
      <div className="isfr__field-color" style={{ background: color }} />
      <span className="isfr__field-name">{label}</span>
      <span className="isfr__field-weight">{weight}%</span>
      <span className="isfr__field-val">{noSources ? 'N/A' : formatBps(Math.round(anim))}</span>
      <span className="isfr__field-pct">{noSources ? '' : formatPercent(Math.round(anim))}</span>
      {noSources ? <div className="isfr__spark-empty" /> : <MiniSparkline data={sparkline} color={color} />}
    </div>
  );
}

function MiniSparkline({ data, color }: { data: number[]; color: string }) {
  if (data.length < 2) return <div className="isfr__spark-empty" />;
  const min = Math.min(...data);
  const max = Math.max(...data);
  const range = max - min || 1;
  const h = 20;
  const w = 60;
  const pts = data.map((v, i) => {
    const x = (i / (data.length - 1)) * w;
    const y = h - ((v - min) / range) * (h - 4) - 2;
    return `${x},${y}`;
  }).join(' ');
  return (
    <svg className="isfr__spark" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none">
      <polyline points={pts} fill="none" stroke={color} strokeWidth="1.5" strokeLinejoin="round" />
    </svg>
  );
}

function SourceTable({ sources, sourceHistory }: {
  sources: IsfrSource[];
  sourceHistory: Record<string, { bps: number; ts: number }[]>;
}) {
  const sorted = useMemo(() => [...sources].sort((a, b) => b.weight - a.weight), [sources]);

  if (!sorted.length) return <div className="isfr__empty">No sources configured</div>;

  const SRC_ACCENT: Record<string, string> = {
    lending: 'var(--rose-bright, #c9a0a8)',
    structured: 'var(--bone-bright, #d8c8a0)',
    funding: 'var(--dream-bright, #61afef)',
    staking: 'var(--green, #98c379)',
  };

  return (
    <div className="isfr__src-scroll">
      {sorted.map((src, i) => {
        const hist = sourceHistory[src.name]?.map((s) => s.bps) ?? [];
        const accent = SRC_ACCENT[src.class] ?? 'var(--border-soft)';
        return (
          <div
            key={src.id}
            className="isfr__src-card"
            style={{ '--i': i, '--src-accent': accent } as React.CSSProperties}
          >
            <span className={`isfr__src-card-dot ${HEALTH_CLS[src.health] ?? 'isfr-dot'}`} />
            <div className="isfr__src-card-info">
              <div className="isfr__src-card-name">{src.name}</div>
              <span className="isfr__src-card-class isfr__src-class" data-class={src.class}>{src.class}</span>
            </div>
            <span className="isfr__src-card-weight">{(src.weight * 100).toFixed(0)}%</span>
            <span className="isfr__src-card-rate">
              {src.lastRateBps != null ? formatBps(src.lastRateBps) : '\u2014'}
            </span>
            <span className="isfr__src-card-spark">
              <MiniSparkline data={hist} color={classColor(src.class)} />
            </span>
          </div>
        );
      })}
    </div>
  );
}

function classColor(cls: string): string {
  switch (cls) {
    case 'lending': return '#98c379';
    case 'structured': return '#61afef';
    case 'staking': return '#e5c07b';
    case 'funding': return '#c9a0a8';
    default: return '#888';
  }
}

function HistoryChart({ history }: { history: IsfrRate[] }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const draw = useCallback((ctx: CanvasRenderingContext2D, w: number, h: number) => {
    ctx.clearRect(0, 0, w, h);

    if (history.length < 2) {
      ctx.fillStyle = 'rgba(200,194,189,0.25)';
      ctx.font = '12px monospace';
      ctx.textAlign = 'center';
      ctx.fillText('Collecting rate samples...', w / 2, h / 2);
      return;
    }

    const pad = { t: 20, b: 28, l: 56, r: 16 };
    const cw = w - pad.l - pad.r;
    const ch = h - pad.t - pad.b;
    const vals = history.map((r) => r.compositeBps);
    const min = Math.min(...vals) * 0.97;
    const max = Math.max(...vals) * 1.03;
    const range = max - min || 1;

    // Grid lines
    ctx.strokeStyle = 'rgba(120,80,96,0.1)';
    ctx.lineWidth = 1;
    ctx.fillStyle = 'rgba(200,194,189,0.3)';
    ctx.font = '10px monospace';
    ctx.textAlign = 'right';
    for (let i = 0; i <= 4; i++) {
      const y = Math.round(pad.t + (i / 4) * ch) + 0.5;
      ctx.beginPath();
      ctx.moveTo(pad.l, y);
      ctx.lineTo(w - pad.r, y);
      ctx.stroke();
      const val = max - (i / 4) * range;
      ctx.fillText(`${Math.round(val)}`, pad.l - 8, y + 3);
    }

    // Time labels on x-axis
    ctx.textAlign = 'center';
    const step = Math.max(1, Math.floor(vals.length / 5));
    for (let i = 0; i < vals.length; i += step) {
      const x = pad.l + (i / (vals.length - 1)) * cw;
      const ts = history[i].timestampMs;
      if (ts > 0) {
        ctx.fillText(fmtTime(ts), x, h - 6);
      }
    }

    // Gradient fill
    ctx.beginPath();
    for (let i = 0; i < vals.length; i++) {
      const x = pad.l + (i / (vals.length - 1)) * cw;
      const y = pad.t + (1 - (vals[i] - min) / range) * ch;
      i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
    }
    const lastX = pad.l + cw;
    ctx.lineTo(lastX, pad.t + ch);
    ctx.lineTo(pad.l, pad.t + ch);
    ctx.closePath();
    const grad = ctx.createLinearGradient(0, pad.t, 0, pad.t + ch);
    grad.addColorStop(0, 'rgba(181,131,141,0.2)');
    grad.addColorStop(1, 'rgba(181,131,141,0)');
    ctx.fillStyle = grad;
    ctx.fill();

    // Line
    ctx.beginPath();
    ctx.strokeStyle = '#b5838d';
    ctx.lineWidth = 2;
    ctx.lineJoin = 'round';
    for (let i = 0; i < vals.length; i++) {
      const x = pad.l + (i / (vals.length - 1)) * cw;
      const y = pad.t + (1 - (vals[i] - min) / range) * ch;
      i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
    }
    ctx.stroke();

    // Latest value marker
    if (vals.length > 0) {
      const lastVal = vals[vals.length - 1];
      const ly = pad.t + (1 - (lastVal - min) / range) * ch;
      ctx.beginPath();
      ctx.arc(lastX, ly, 3, 0, Math.PI * 2);
      ctx.fillStyle = '#b5838d';
      ctx.fill();
      ctx.beginPath();
      ctx.arc(lastX, ly, 6, 0, Math.PI * 2);
      ctx.strokeStyle = 'rgba(181,131,141,0.4)';
      ctx.lineWidth = 1;
      ctx.stroke();
    }
  }, [history]);

  useCanvasSetup(canvasRef, draw, [history]);

  return (
    <div className="isfr__chart-wrap">
      <canvas ref={canvasRef} style={{ width: '100%', height: '100%', display: 'block' }} />
    </div>
  );
}

function EventLog({ events }: { events: IsfrEventEntry[] }) {
  const logRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);

  useEffect(() => {
    if (autoScroll && logRef.current) {
      logRef.current.scrollTop = 0;
    }
  }, [events.length, autoScroll]);

  const handleScroll = useCallback(() => {
    if (!logRef.current) return;
    setAutoScroll(logRef.current.scrollTop < 10);
  }, []);

  if (!events.length) return <div className="isfr__empty">Waiting for ISFR events...</div>;

  return (
    <div className="isfr__log-scroll" ref={logRef} onScroll={handleScroll}>
      {events.slice(0, 100).map((ev, i) => {
        const t = ev.type ?? 'source';
        const color = EVT_CLR[t] ?? 'var(--text-ghost)';
        return (
          <div key={`${ev.ts}-${i}`} className="isfr__log-row">
            <span className="isfr__log-dot" style={{ background: color }} />
            <span className="isfr__log-time">{fmtTime(ev.ts)}</span>
            <span className="isfr__log-type" style={{ color }}>{t}</span>
            <span className="isfr__log-msg">{ev.message}</span>
          </div>
        );
      })}
    </div>
  );
}

// ── Feed Agent subcomponents ──────────────────────────────────

function AgentList({ agents, feeds }: { agents: RelayAgentEntry[]; feeds: RelayFeed[] }) {
  const sorted = useMemo(() => [...agents].sort((a, b) => a.name.localeCompare(b.name)), [agents]);

  if (!sorted.length) return <div className="isfr__empty">No feed agents connected</div>;

  return (
    <div className="isfr__agent-scroll">
      {sorted.map((a) => {
        const agentFeeds = feeds.filter((f) => f.feedId.startsWith(a.agentId.slice(0, 8)));
        const ago = Math.floor((Date.now() - a.connectedAtMs) / 1000);
        const agoLabel = ago < 60 ? `${ago}s` : ago < 3600 ? `${Math.floor(ago / 60)}m` : `${Math.floor(ago / 3600)}h`;
        return (
          <div key={a.agentId} className="isfr__agent-card">
            <div className="isfr__agent-top">
              <span className="isfr-dot isfr-dot--live" />
              <span className="isfr__agent-name">{a.name || a.agentId.slice(0, 12)}</span>
              <span className="isfr__agent-ago">{agoLabel}</span>
            </div>
            {a.capabilities.length > 0 && (
              <div className="isfr__agent-caps">
                {a.capabilities.map((c) => (
                  <span key={c} className="isfr__agent-cap">{c}</span>
                ))}
              </div>
            )}
            {agentFeeds.length > 0 && (
              <div className="isfr__agent-feeds">
                {agentFeeds.map((f) => (
                  <span key={f.feedId} className="isfr__agent-feed">{f.topic}</span>
                ))}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function FeedLog({ entries }: { entries: FeedLogEntry[] }) {
  const logRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);

  useEffect(() => {
    if (autoScroll && logRef.current) {
      logRef.current.scrollTop = 0;
    }
  }, [entries.length, autoScroll]);

  const handleScroll = useCallback(() => {
    if (!logRef.current) return;
    setAutoScroll(logRef.current.scrollTop < 10);
  }, []);

  if (!entries.length) return <div className="isfr__empty">Waiting for feed data...</div>;

  return (
    <div className="isfr__feedlog-scroll" ref={logRef} onScroll={handleScroll}>
      {entries.slice(0, 200).map((e, i) => (
        <div key={`${e.ts}-${i}`} className="isfr__feedlog-row">
          <span className="isfr__feedlog-time">{fmtTime(e.ts)}</span>
          <span className="isfr__feedlog-topic">{e.topic}</span>
          <span className="isfr__feedlog-preview">{e.preview}</span>
        </div>
      ))}
    </div>
  );
}

// ── Chain Explorer subcomponents ──────────────────────────────

function truncHash(hash: string): string {
  if (hash.length <= 12) return hash;
  return `${hash.slice(0, 6)}\u2026${hash.slice(-4)}`;
}

function truncAddr(addr: string): string {
  if (addr.length <= 10) return addr;
  return `${addr.slice(0, 6)}\u2026${addr.slice(-4)}`;
}

function formatWei(wei: string): string {
  const n = BigInt(wei);
  if (n === 0n) return '0';
  const eth = Number(n) / 1e18;
  if (eth < 0.001) return '<0.001 ETH';
  return `${eth.toFixed(3)} ETH`;
}

function formatGas(gas: number): string {
  if (gas >= 1_000_000) return `${(gas / 1_000_000).toFixed(1)}M`;
  if (gas >= 1_000) return `${(gas / 1_000).toFixed(0)}k`;
  return String(gas);
}

function ChainBlockFeed({ blocks }: { blocks: ChainBlockEntry[] }) {
  if (!blocks.length) return <div className="isfr__empty">Waiting for blocks...</div>;

  return (
    <div className="isfr__chain-scroll">
      {blocks.map((b) => {
        const ago = Math.floor((Date.now() / 1000) - b.timestamp);
        const gasPercent = b.gasLimit > 0 ? (b.gasUsed / b.gasLimit) * 100 : 0;
        return (
          <div key={b.number} className="isfr__block-row">
            <span className="isfr__block-num">#{b.number}</span>
            <span className="isfr__block-hash" title={b.hash}>{truncHash(b.hash)}</span>
            <span className="isfr__block-ago">{ago}s</span>
            <div className="isfr__block-gas-bar">
              <div className="isfr__block-gas-fill" style={{ width: `${gasPercent}%` }} />
            </div>
            <span className="isfr__block-txcount">{b.txCount}tx</span>
          </div>
        );
      })}
    </div>
  );
}

function ChainTxFeed({ txs }: { txs: ChainTxEntry[] }) {
  if (!txs.length) return <div className="isfr__empty">No transactions yet</div>;

  return (
    <div className="isfr__chain-scroll">
      {txs.map((tx, i) => {
        const isCreate = tx.to === null;
        return (
          <div key={`${tx.txHash}-${i}`}
            className={`isfr__tx-row${!tx.success ? ' isfr__tx-row--fail' : ''}${isCreate ? ' isfr__tx-row--create' : ''}`}>
            <span className="isfr__tx-block">#{tx.blockNumber}</span>
            <span className="isfr__tx-addrs">
              {truncAddr(tx.from)} → {isCreate ? 'CREATE' : truncAddr(tx.to!)}
            </span>
            <span className="isfr__tx-value">{formatWei(tx.valueWei)}</span>
            <span className="isfr__tx-gas">{formatGas(tx.gasUsed)}</span>
            <span className="isfr__tx-method">{tx.methodSig ?? '\u2014'}</span>
          </div>
        );
      })}
    </div>
  );
}

function ChainEventFeed({ events }: { events: ChainEventEntry[] }) {
  if (!events.length) return <div className="isfr__empty">No contract events yet</div>;

  return (
    <div className="isfr__chain-scroll">
      {events.map((evt, i) => {
        const params = Object.entries(evt.decoded)
          .map(([k, v]) => `${k}=${String(v)}`)
          .join(' ');
        const colorCls = evt.eventName === 'RateSubmitted' ? 'isfr__cev--rate' :
          evt.eventName === 'KeeperRewarded' ? 'isfr__cev--reward' : 'isfr__cev--default';
        return (
          <div key={`${evt.txHash}-${evt.logIndex}-${i}`} className={`isfr__cev-row ${colorCls}`}>
            <span className="isfr__cev-name">{evt.eventName}</span>
            <span className="isfr__cev-params" title={params}>{params || '\u2014'}</span>
            <span className="isfr__cev-block">#{evt.blockNumber}</span>
          </div>
        );
      })}
    </div>
  );
}
