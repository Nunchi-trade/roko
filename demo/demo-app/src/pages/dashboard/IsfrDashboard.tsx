import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { useDataHub } from '../../app/DataHub';
import type { IsfrEventEntry } from '../../app/DataHub';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
import { useCountUp } from '../../hooks/useCountUp';
import { useCanvasSetup } from '../../hooks/useCanvasSetup';
import Oscilloscope from '../../components/canvas/Oscilloscope';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import {
  type IsfrStatus,
  formatBps,
  formatPercent,
} from '../../lib/isfr-api';
import { stripAnsi } from '../../lib/strip-ansi';
import { WS_BASE } from '../../lib/serve-url';
import { useLiveApi } from '../../hooks/useLiveApi';
import ChainTab from './ChainTab';
import '../dashboard/dashboard.css';
import './IsfrDashboard.css';

/* ── Constants ───────────────────────────────────────────────── */

const CLASS_COLORS: Record<string, string> = {
  lending:    'var(--status-active)',
  structured: 'var(--status-blocked)',
  staking:    'var(--bone-bright)',
  funding:    'var(--dream-bright)',
};

const HEALTH_COLORS: Record<string, string> = {
  live:    'var(--success)',
  stale:   'var(--warning)',
  offline: 'var(--rose-bright)',
};

type TabId = 'sources' | 'history' | 'events' | 'agents' | 'onchain' | 'chain';

/* ── Main component ──────────────────────────────────────────── */

export default function IsfrDashboard() {
  const { get } = useLiveApi();
  const [tab, setTab] = useState<TabId>('sources');
  const [status, setStatus] = useState<IsfrStatus | null>(null);
  const [tickerFlash, setTickerFlash] = useState(false);
  const [tickAgo, setTickAgo] = useState(0);
  const [pollCountdown, setPollCountdown] = useState(0);
  const [startTime] = useState(Date.now());
  const [uptime, setUptime] = useState('0s');
  const lastEventTsRef = useRef(0);
  const [initialLoading, setInitialLoading] = useState(true);

  // DataHub selectors
  const currentRate = useDataHub((s) => s.isfrCurrentRate);
  const isfrHistory = useDataHub((s) => s.isfrHistory);
  const sources = useDataHub((s) => s.isfrSources);
  const keeperStatus = useDataHub((s) => s.isfrKeeperStatus);
  const fieldHistory = useDataHub((s) => s.isfrFieldHistory);
  const sourceHistory = useDataHub((s) => s.isfrSourceHistory);
  const eventLog = useDataHub((s) => s.isfrEventLog);
  const readingsCache = useDataHub((s) => s.isfrReadingsCache);
  const fetchIsfrStatus = useDataHub((s) => s.fetchIsfrStatus);
  const fetchIsfrCurrent = useDataHub((s) => s.fetchIsfrCurrent);
  const fetchIsfrHistory = useDataHub((s) => s.fetchIsfrHistory);
  const fetchIsfrSources = useDataHub((s) => s.fetchIsfrSources);

  const compositeBps = currentRate?.compositeBps ?? 0;
  const confidenceBps = currentRate?.confidenceBps ?? 0;
  const confidencePct = confidenceBps / 100;
  const sourceCount = currentRate?.sourceCount ?? 0;
  const keeperRunning = keeperStatus === 'running';
  const healthyCt = sources.filter((s) => s.health === 'live').length;

  // Previous composite for delta
  const prevCompositeRef = useRef(compositeBps);
  const [compositeDelta, setCompositeDelta] = useState(0);
  useEffect(() => {
    if (currentRate && prevCompositeRef.current !== compositeBps) {
      setCompositeDelta(compositeBps - prevCompositeRef.current);
      prevCompositeRef.current = compositeBps;
    }
  }, [compositeBps, currentRate]);

  // Animated counter
  const animComposite = useCountUp(compositeBps, 900);

  // Initial fetch
  const fetchAll = useCallback(async () => {
    try {
      const statusPromise = get<IsfrStatus>('/api/isfr/status');
      await Promise.all([
        fetchIsfrCurrent(),
        fetchIsfrHistory(256),
        fetchIsfrSources(),
        fetchIsfrStatus(),
      ]);
      const s = await statusPromise;
      setStatus(s);
    } catch {
      // keep stale
    }
  }, [get, fetchIsfrCurrent, fetchIsfrHistory, fetchIsfrSources, fetchIsfrStatus]);

  useEffect(() => {
    fetchAll().finally(() => setInitialLoading(false));
    const id = setInterval(fetchAll, 30_000);
    return () => clearInterval(id);
  }, [fetchAll]);

  // SSE-triggered refetch
  const debouncedRefetch = useDebouncedRefetch(fetchAll, 2000);
  useContextEventSubscription(
    ['isfr_rate_computed', 'isfr_source_health_changed', 'isfr_keeper_state_changed'],
    useCallback(() => {
      debouncedRefetch();
      lastEventTsRef.current = Date.now();
      // Flash ticker
      setTickerFlash(true);
      setTimeout(() => setTickerFlash(false), 600);
      // Reset poll countdown
      setPollCountdown(status?.poll_interval_secs ?? 10);
    }, [debouncedRefetch, status?.poll_interval_secs]),
  );

  // 1-second interval for freshness, poll countdown, and uptime
  useEffect(() => {
    const id = setInterval(() => {
      const now = Date.now();
      if (lastEventTsRef.current > 0) {
        setTickAgo(Math.floor((now - lastEventTsRef.current) / 1000));
      }
      setPollCountdown((c) => Math.max(0, c - 1));
      const elapsed = Math.floor((now - startTime) / 1000);
      const m = Math.floor(elapsed / 60);
      const s = elapsed % 60;
      setUptime(m > 0 ? `${m}m ${s}s` : `${s}s`);
    }, 1000);
    return () => clearInterval(id);
  }, [startTime]);

  // Seed poll countdown from status
  useEffect(() => {
    if (status?.poll_interval_secs) {
      setPollCountdown(status.poll_interval_secs);
    }
  }, [status?.poll_interval_secs]);

  // Tab visibility
  const showOnChain = false; // placeholder: no oracle yet
  const TABS: { id: TabId; label: string; hidden?: boolean }[] = [
    { id: 'sources', label: 'Sources' },
    { id: 'history', label: 'History' },
    { id: 'events', label: 'Events' },
    { id: 'agents', label: 'Agents' },
    { id: 'chain', label: 'Chain' },
    { id: 'onchain', label: 'On-Chain', hidden: !showOnChain },
  ];

  // Class BPS for contribution bar
  const classBps = useMemo(() => {
    const lending = currentRate?.lendingBps ?? 0;
    const structured = currentRate?.structuredBps ?? 0;
    const staking = currentRate?.stakingBps ?? 0;
    const funding = currentRate?.fundingBps ?? 0;
    const total = lending + structured + staking + funding;
    return { lending, structured, staking, funding, total };
  }, [currentRate]);

  if (initialLoading) {
    return (
      <div className="dash-page progressive-reveal cd-skeleton-layout">
        <div className="skeleton cd-skeleton-hero" />
        <div className="cd-skeleton-grid">
          <div className="skeleton-card skeleton" />
          <div className="skeleton-card skeleton" />
          <div className="skeleton-card skeleton" />
        </div>
      </div>
    );
  }

  return (
    <div className="dash-page">
      {/* ── TICKER BAR ───────────────────────────────── */}
      <div className={`isfr-ticker${tickerFlash ? ' isfr-ticker--flash' : ''}`}>
        <div className="isfr-ticker__left">
          {keeperRunning && (
            <div className="isfr-ticker__live">
              <span className="isfr-ticker__live-dot" />
              LIVE
            </div>
          )}
          <span>ISFR RATE FEED</span>
          <span style={{ color: 'var(--text-ghost)' }}>running {uptime}</span>
        </div>
        <div className="isfr-ticker__center">
          <span className="isfr-ticker__rate">{Math.round(animComposite)} bps</span>
          {compositeDelta !== 0 && (
            <span className={`isfr-ticker__trend isfr-ticker__trend--${compositeDelta > 0 ? 'up' : 'down'}`}>
              {compositeDelta > 0 ? '\u25b2' : '\u25bc'} {compositeDelta > 0 ? '+' : ''}{compositeDelta}
            </span>
          )}
          {compositeDelta === 0 && currentRate && (
            <span className="isfr-ticker__trend isfr-ticker__trend--flat">{'\u2014'}</span>
          )}
        </div>
        <div className="isfr-ticker__right">
          <span className="isfr-ticker__freshness">
            tick {tickAgo > 0 ? `${tickAgo}s ago` : 'now'}
          </span>
          <span className="isfr-ticker__epoch">E:0</span>
        </div>
      </div>

      {/* ── OSCILLOSCOPE ──────────────────────────────── */}
      <div style={{ marginTop: 4 }}>
        <Oscilloscope data={fieldHistory.composite} height={100} />
      </div>

      {/* ── RATE MOSAIC ──────────────────────────────── */}
      <div className="isfr-mosaic-wrap dash-stagger" style={{ '--stagger-i': 0 } as React.CSSProperties}>
        <Mosaic columns={6}>
          <MosaicCell
            label="COMPOSITE"
            value={formatBps(Math.round(animComposite))}
            color="rose"
            mono
            sub={formatPercent(compositeBps)}
            sparkline={fieldHistory.composite}
          />
          <MosaicCell
            label="LENDING"
            value={formatBps(currentRate?.lendingBps ?? null)}
            color="teal"
            mono
            sub={deltaStr(fieldHistory.lending)}
            sparkline={fieldHistory.lending}
          />
          <MosaicCell
            label="STRUCTURED"
            value={formatBps(currentRate?.structuredBps ?? null)}
            color="violet"
            mono
            sub={deltaStr(fieldHistory.structured)}
            sparkline={fieldHistory.structured}
          />
          <MosaicCell
            label="FUNDING"
            value={formatBps(currentRate?.fundingBps ?? null)}
            color="dream"
            mono
            sub={currentRate?.fundingBps ? deltaStr(fieldHistory.funding) : 'no sources'}
            sparkline={fieldHistory.funding}
          />
          <MosaicCell
            label="STAKING"
            value={formatBps(currentRate?.stakingBps ?? null)}
            color="bone"
            mono
            sub={deltaStr(fieldHistory.staking)}
            sparkline={fieldHistory.staking}
          />
          <MosaicCell
            label="CONFIDENCE"
            value={<ConfidenceGauge pct={confidencePct} />}
            color="success"
            sub={`${healthyCt}/${sourceCount} live`}
          />
        </Mosaic>
      </div>

      {/* ── VISUAL METERS ────────────────────────────── */}
      <div className="isfr-meters dash-stagger" style={{ '--stagger-i': 1 } as React.CSSProperties}>
        <div className="isfr-meters__left">
          <div className="isfr-contrib-bar">
            {classBps.total > 0 ? (
              <>
                {classBps.lending > 0 && (
                  <div
                    className="isfr-contrib-seg"
                    style={{
                      flexBasis: `${(classBps.lending / classBps.total) * 100}%`,
                      background: CLASS_COLORS.lending,
                    }}
                  >
                    {classBps.lending / classBps.total > 0.15 ? `${Math.round((classBps.lending / classBps.total) * 100)}%` : ''}
                  </div>
                )}
                {classBps.structured > 0 && (
                  <div
                    className="isfr-contrib-seg"
                    style={{
                      flexBasis: `${(classBps.structured / classBps.total) * 100}%`,
                      background: CLASS_COLORS.structured,
                    }}
                  >
                    {classBps.structured / classBps.total > 0.15 ? `${Math.round((classBps.structured / classBps.total) * 100)}%` : ''}
                  </div>
                )}
                {classBps.staking > 0 && (
                  <div
                    className="isfr-contrib-seg"
                    style={{
                      flexBasis: `${(classBps.staking / classBps.total) * 100}%`,
                      background: CLASS_COLORS.staking,
                    }}
                  >
                    {classBps.staking / classBps.total > 0.15 ? `${Math.round((classBps.staking / classBps.total) * 100)}%` : ''}
                  </div>
                )}
                {classBps.funding > 0 && (
                  <div
                    className="isfr-contrib-seg"
                    style={{
                      flexBasis: `${(classBps.funding / classBps.total) * 100}%`,
                      background: CLASS_COLORS.funding,
                    }}
                  >
                    {classBps.funding / classBps.total > 0.15 ? `${Math.round((classBps.funding / classBps.total) * 100)}%` : ''}
                  </div>
                )}
              </>
            ) : (
              <div className="isfr-contrib-seg" style={{ flexBasis: '100%', background: 'var(--glass-border)' }} />
            )}
          </div>
          <div className="isfr-contrib-labels">
            <ContribLabel color={CLASS_COLORS.lending} label="lending" pct={classBps.total > 0 ? Math.round((classBps.lending / classBps.total) * 100) : 0} />
            <ContribLabel color={CLASS_COLORS.structured} label="structured" pct={classBps.total > 0 ? Math.round((classBps.structured / classBps.total) * 100) : 0} />
            <ContribLabel color={CLASS_COLORS.staking} label="staking" pct={classBps.total > 0 ? Math.round((classBps.staking / classBps.total) * 100) : 0} />
            <ContribLabel color={CLASS_COLORS.funding} label="funding" pct={classBps.total > 0 ? Math.round((classBps.funding / classBps.total) * 100) : 0} />
          </div>
        </div>
        <div className="isfr-meters__right">
          <EpochRing epochNumber={0} pollInterval={status?.poll_interval_secs ?? 10} countdown={pollCountdown} />
        </div>
      </div>

      {/* ── TAB BAR ──────────────────────────────────── */}
      <div className="isfr-tab-bar">
        {TABS.filter((t) => !t.hidden).map((t) => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            className={`isfr-tab-btn${tab === t.id ? ' isfr-tab-btn--active' : ''}`}
          >
            {t.label}
          </button>
        ))}
      </div>

      {/* ── TAB CONTENT ──────────────────────────────── */}
      <div key={tab} className="isfr-tab-content-enter">
        {tab === 'sources' ? (
          <SourcesTab
            sources={sources}
            sourceHistory={sourceHistory}
            readingsCache={readingsCache}
            compositeBps={compositeBps}
          />
        ) : tab === 'history' ? (
          <HistoryTab history={isfrHistory} />
        ) : tab === 'events' ? (
          <EventsTab eventLog={eventLog} />
        ) : tab === 'agents' ? (
          <AgentsTab />
        ) : tab === 'chain' ? (
          <ChainTab />
        ) : tab === 'onchain' ? (
          <OnChainTab />
        ) : null}
      </div>
    </div>
  );
}

/* ── Confidence Gauge ────────────────────────────────────────── */

function ConfidenceGauge({ pct }: { pct: number }) {
  const r = 24;
  const circ = 2 * Math.PI * r;
  const offset = circ - (Math.min(pct, 100) / 100) * circ;
  const color =
    pct >= 80 ? 'var(--success)' : pct >= 50 ? 'var(--warning)' : 'var(--rose-bright)';

  return (
    <div className="isfr-gauge-wrap">
      <svg width={60} height={60} className="isfr-gauge-svg" viewBox="0 0 60 60">
        <circle className="isfr-gauge-bg" cx={30} cy={30} r={r} />
        <circle
          className="isfr-gauge-fill"
          cx={30}
          cy={30}
          r={r}
          stroke={color}
          strokeDasharray={circ}
          strokeDashoffset={offset}
        />
        <text className="isfr-gauge-label" x={30} y={30}>
          {Math.round(pct)}%
        </text>
      </svg>
    </div>
  );
}

/* ── Contribution label helper ───────────────────────────────── */

function ContribLabel({ color, label, pct }: { color: string; label: string; pct: number }) {
  return (
    <span className="isfr-contrib-label">
      <span className="isfr-contrib-dot" style={{ background: color }} />
      {label} {pct}%
    </span>
  );
}

/* ── Epoch Ring ──────────────────────────────────────────────── */

function EpochRing({ epochNumber, pollInterval, countdown }: { epochNumber: number; pollInterval: number; countdown: number }) {
  const r = 22;
  const circ = 2 * Math.PI * r;
  const progress = pollInterval > 0 ? (pollInterval - countdown) / pollInterval : 0;
  const offset = circ - progress * circ;

  return (
    <div className="isfr-epoch-ring">
      <svg width={56} height={56} className="isfr-epoch-svg" viewBox="0 0 56 56">
        <circle className="isfr-epoch-bg" cx={28} cy={28} r={r} />
        <circle
          className="isfr-epoch-fill"
          cx={28}
          cy={28}
          r={r}
          strokeDasharray={circ}
          strokeDashoffset={offset}
        />
        <text className="isfr-epoch-label" x={28} y={28}>
          E:{epochNumber}
        </text>
      </svg>
      <div className="isfr-poll-countdown">
        next tick in {countdown}s
      </div>
      <div className="isfr-poll-track">
        <div
          className="isfr-poll-fill"
          style={{ width: pollInterval > 0 ? `${(1 - countdown / pollInterval) * 100}%` : '0%' }}
        />
      </div>
    </div>
  );
}

/* ── Sources Tab ─────────────────────────────────────────────── */

function SourcesTab({
  sources,
  sourceHistory,
  readingsCache,
  compositeBps,
}: {
  sources: Array<{
    id: string;
    name: string;
    class: string;
    weight: number;
    lastRateBps: number | null;
    health: 'live' | 'stale' | 'offline';
    lastPollMs: number | null;
  }>;
  sourceHistory: Record<string, Array<{ bps: number; ts: number }>>;
  readingsCache: Record<string, Record<string, unknown>>;
  compositeBps: number;
}) {
  const [expandedId, setExpandedId] = useState<string | null>(null);

  if (sources.length === 0) {
    return (
      <div className="dash-placeholder">No sources registered</div>
    );
  }

  return (
    <div className="isfr-source-grid dash-stagger" style={{ '--stagger-i': 2 } as React.CSSProperties}>
      {sources.map((src, idx) => {
        const history = sourceHistory[src.name] ?? [];
        const historyBps = history.map((h) => h.bps);
        const meta = readingsCache[src.name] ?? null;
        const expanded = expandedId === src.id;
        const contribution = src.lastRateBps != null && compositeBps > 0
          ? ((src.weight * src.lastRateBps) / compositeBps * 100)
          : null;
        const stats = historyBps.length > 0
          ? {
              min: Math.min(...historyBps),
              max: Math.max(...historyBps),
              avg: Math.round(historyBps.reduce((a, b) => a + b, 0) / historyBps.length),
            }
          : null;

        return (
          <div
            key={src.id}
            className="isfr-source-card"
            style={{ '--card-i': idx } as React.CSSProperties}
            onClick={() => setExpandedId(expanded ? null : src.id)}
          >
            {/* Row 1: icon, name, class badge, weight bar, health dot */}
            <div className="isfr-source-card__row1">
              <div
                className="isfr-source-card__icon"
                style={{ background: CLASS_COLORS[src.class] ?? 'var(--text-ghost)' }}
              >
                {src.name.charAt(0).toUpperCase()}
              </div>
              <span className="isfr-source-card__name">{src.name}</span>
              <span
                className="isfr-source-card__class"
                style={{
                  background: `color-mix(in srgb, ${CLASS_COLORS[src.class] ?? 'var(--text-ghost)'} 20%, transparent)`,
                  color: CLASS_COLORS[src.class] ?? 'var(--text-ghost)',
                  border: `1px solid color-mix(in srgb, ${CLASS_COLORS[src.class] ?? 'var(--text-ghost)'} 30%, transparent)`,
                }}
              >
                {src.class}
              </span>
              <div className="isfr-source-card__weight">
                <div
                  className="isfr-source-card__weight-fill"
                  style={{
                    width: `${src.weight * 100}%`,
                    background: CLASS_COLORS[src.class] ?? 'var(--text-ghost)',
                  }}
                />
              </div>
              <div
                className="isfr-source-card__health-dot"
                style={{
                  background: HEALTH_COLORS[src.health] ?? 'var(--text-ghost)',
                  boxShadow: src.health === 'live' ? `0 0 6px ${HEALTH_COLORS[src.health]}` : 'none',
                  animation: src.health === 'live' ? 'pulse-dot 2s ease-in-out infinite' : 'none',
                }}
              />
            </div>

            {/* Row 2: rate, delta, freshness, mini sparkline */}
            <div className="isfr-source-card__row2">
              <span className="isfr-source-card__bps">
                {formatBps(src.lastRateBps)}
              </span>
              {historyBps.length >= 2 && (
                <SourceDelta current={historyBps[historyBps.length - 1]} prev={historyBps[historyBps.length - 2]} />
              )}
              <span className="isfr-source-card__freshness">
                {src.lastPollMs ? `polled ${Math.floor((Date.now() - src.lastPollMs) / 1000)}s ago` : ''}
              </span>
              {historyBps.length >= 2 && (
                <MiniSparkline data={historyBps} color={CLASS_COLORS[src.class] ?? '#888'} />
              )}
            </div>

            {/* Expandable detail */}
            <div className={`isfr-source-card__expand${expanded ? ' isfr-source-card__expand--open' : ''}`}>
              <div className="isfr-source-card__detail">
                {meta && (
                  <div className="isfr-source-card__meta-grid">
                    {Object.entries(meta).map(([k, v]) => (
                      <MetaRow key={k} label={k} value={v} />
                    ))}
                  </div>
                )}
                {contribution != null && (
                  <div className="isfr-source-card__stats" style={{ marginTop: meta ? 6 : 0 }}>
                    Contribution: <span>{contribution.toFixed(1)}%</span> of composite
                  </div>
                )}
                {stats && (
                  <div className="isfr-source-card__stats">
                    min: <span>{stats.min}</span> &middot; max: <span>{stats.max}</span> &middot; avg: <span>{stats.avg}</span>
                    {historyBps.length > 0 && <> &middot; ({historyBps.length} ticks)</>}
                  </div>
                )}
                {historyBps.length >= 2 && (
                  <div className="isfr-source-card__chart">
                    <SourceSparkChart data={historyBps} color={CLASS_COLORS[src.class] ?? '#888'} />
                  </div>
                )}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

/* ── Source delta badge ───────────────────────────────────────── */

function SourceDelta({ current, prev }: { current: number; prev: number }) {
  const d = current - prev;
  if (d === 0) return null;
  const cls = d > 0 ? 'isfr-ticker__trend--up' : 'isfr-ticker__trend--down';
  return (
    <span className={`isfr-source-card__delta ${cls}`}>
      {d > 0 ? '+' : ''}{d}
    </span>
  );
}

/* ── Meta row ────────────────────────────────────────────────── */

function MetaRow({ label, value }: { label: string; value: unknown }) {
  const display = typeof value === 'string' ? value : JSON.stringify(value);
  return (
    <>
      <span className="isfr-source-card__meta-key">{label}</span>
      <span className="isfr-source-card__meta-val" title={display}>{display}</span>
    </>
  );
}

/* ── Mini sparkline (inline canvas) ──────────────────────────── */

function MiniSparkline({ data, color }: { data: number[]; color: string }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const resolvedColor = useResolvedColor(color);

  useCanvasSetup(
    canvasRef,
    (ctx, w, h) => {
      ctx.clearRect(0, 0, w, h);
      if (data.length < 2) return;
      const min = Math.min(...data);
      const max = Math.max(...data);
      const range = max - min || 1;
      ctx.beginPath();
      ctx.strokeStyle = resolvedColor;
      ctx.lineWidth = 1.5;
      ctx.lineJoin = 'round';
      for (let i = 0; i < data.length; i++) {
        const x = (i / (data.length - 1)) * w;
        const y = h - ((data[i] - min) / range) * (h - 2) - 1;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.stroke();
    },
    [data, resolvedColor],
  );

  return (
    <canvas
      ref={canvasRef}
      className="isfr-source-card__spark"
      style={{ width: 80, height: 20 }}
    />
  );
}

/* ── Source sparkline chart (expanded card) ───────────────────── */

function SourceSparkChart({ data, color }: { data: number[]; color: string }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const resolvedColor = useResolvedColor(color);

  useCanvasSetup(
    canvasRef,
    (ctx, w, h) => {
      ctx.clearRect(0, 0, w, h);
      if (data.length < 2) return;
      const min = Math.min(...data) - 5;
      const max = Math.max(...data) + 5;
      const range = max - min || 1;
      const pad = 2;

      // Gradient fill
      ctx.beginPath();
      for (let i = 0; i < data.length; i++) {
        const x = pad + (i / (data.length - 1)) * (w - 2 * pad);
        const y = pad + (1 - (data[i] - min) / range) * (h - 2 * pad);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.lineTo(w - pad, h - pad);
      ctx.lineTo(pad, h - pad);
      ctx.closePath();
      ctx.fillStyle = hexToRgba(resolvedColor, 0.1);
      ctx.fill();

      // Line
      ctx.beginPath();
      ctx.strokeStyle = resolvedColor;
      ctx.lineWidth = 1.5;
      ctx.lineJoin = 'round';
      for (let i = 0; i < data.length; i++) {
        const x = pad + (i / (data.length - 1)) * (w - 2 * pad);
        const y = pad + (1 - (data[i] - min) / range) * (h - 2 * pad);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.stroke();
    },
    [data, resolvedColor],
  );

  return (
    <div style={{ width: '100%', height: '100%' }}>
      <canvas ref={canvasRef} style={{ width: '100%', height: '100%', display: 'block' }} />
    </div>
  );
}

/* ── History Tab ─────────────────────────────────────────────── */

function HistoryTab({ history }: { history: Array<{ compositeBps: number; lendingBps: number; structuredBps: number; fundingBps: number; stakingBps: number; timestampMs: number }> }) {
  const [range, setRange] = useState(60);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const tooltipRef = useRef<HTMLDivElement>(null);

  const sliced = useMemo(() => history.slice(-range), [history, range]);

  const lineColors = useMemo(() => ({
    composite: getCssVar('--rose-bright'),
    lending: getCssVar('--status-active'),
    structured: getCssVar('--status-blocked'),
    staking: getCssVar('--bone-bright'),
  }), []);

  // Interactive crosshair
  const handleMouseMove = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    const tooltip = tooltipRef.current;
    const canvas = canvasRef.current;
    if (!tooltip || !canvas || sliced.length < 2) {
      if (tooltip) tooltip.style.opacity = '0';
      return;
    }
    const w = rect.width;
    const pad = { left: 44, right: 12, top: 16, bottom: 24 };
    const plotW = w - pad.left - pad.right;
    const idx = Math.round(((mx - pad.left) / plotW) * (sliced.length - 1));
    if (idx < 0 || idx >= sliced.length) {
      tooltip.style.opacity = '0';
      return;
    }
    const pt = sliced[idx];
    const ago = Math.round((Date.now() - pt.timestampMs) / 1000);
    tooltip.innerHTML = [
      `<span style="color:${lineColors.composite}">Composite: ${pt.compositeBps} bps</span>`,
      `<span style="color:${lineColors.lending}">Lending: ${pt.lendingBps} bps</span>`,
      `<span style="color:${lineColors.structured}">Structured: ${pt.structuredBps} bps</span>`,
      `<span style="color:${lineColors.staking}">Staking: ${pt.stakingBps} bps</span>`,
      `<span style="color:var(--text-ghost)">${ago}s ago</span>`,
    ].join('<br>');
    tooltip.style.opacity = '1';
    const tx = Math.min(mx + 12, w - 180);
    tooltip.style.left = `${tx}px`;
    tooltip.style.top = `${Math.max(my - 60, 8)}px`;
  }, [sliced, lineColors]);

  const handleMouseLeave = useCallback(() => {
    if (tooltipRef.current) tooltipRef.current.style.opacity = '0';
  }, []);

  useCanvasSetup(
    canvasRef,
    (ctx, w, h) => {
      const pad = { top: 16, right: 12, bottom: 24, left: 44 };
      const plotW = w - pad.left - pad.right;
      const plotH = h - pad.top - pad.bottom;

      ctx.clearRect(0, 0, w, h);

      if (sliced.length < 2) {
        ctx.fillStyle = getCssVar('--text-ghost');
        ctx.font = '10px "JetBrains Mono", monospace';
        ctx.fillText('Waiting for rate data\u2026', pad.left, pad.top + 20);
        return;
      }

      // Compute Y range across all lines
      const allVals = sliced.flatMap((r) => [r.compositeBps, r.lendingBps, r.structuredBps, r.stakingBps]);
      const min = Math.min(...allVals) - 20;
      const max = Math.max(...allVals) + 20;
      const yRange = max - min || 1;

      // Grid lines
      ctx.strokeStyle = 'rgba(255,255,255,0.05)';
      ctx.lineWidth = 1;
      ctx.fillStyle = getCssVar('--text-ghost');
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'right';
      for (let i = 0; i <= 4; i++) {
        const yy = pad.top + plotH * (1 - i / 4);
        ctx.beginPath();
        ctx.setLineDash([3, 3]);
        ctx.moveTo(pad.left, yy);
        ctx.lineTo(pad.left + plotW, yy);
        ctx.stroke();
        ctx.setLineDash([]);
        const label = Math.round(min + (yRange * i) / 4);
        ctx.fillText(`${label}`, pad.left - 6, yy + 3);
      }

      // X-axis time labels
      ctx.textAlign = 'center';
      ctx.fillStyle = getCssVar('--text-ghost');
      const now = Date.now();
      for (let i = 0; i < sliced.length; i += Math.max(1, Math.floor(sliced.length / 5))) {
        const x = pad.left + (i / (sliced.length - 1)) * plotW;
        const ago = Math.round((now - sliced[i].timestampMs) / 1000);
        const label = ago >= 60 ? `${Math.floor(ago / 60)}m` : `${ago}s`;
        ctx.fillText(label, x, h - 4);
      }

      // Draw lines helper
      const drawLine = (
        field: keyof typeof sliced[0],
        color: string,
        lineWidth: number,
        fill: boolean,
      ) => {
        const vals = sliced.map((r) => r[field] as number);
        ctx.beginPath();
        ctx.strokeStyle = color;
        ctx.lineWidth = lineWidth;
        ctx.lineJoin = 'round';
        ctx.lineCap = 'round';
        for (let i = 0; i < vals.length; i++) {
          const x = pad.left + (i / (vals.length - 1)) * plotW;
          const y = pad.top + plotH - ((vals[i] - min) / yRange) * plotH;
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.stroke();

        if (fill) {
          ctx.lineTo(pad.left + plotW, pad.top + plotH);
          ctx.lineTo(pad.left, pad.top + plotH);
          ctx.closePath();
          ctx.fillStyle = hexToRgba(color, 0.08);
          ctx.fill();
        }

        // Endpoint dot
        const lastX = pad.left + plotW;
        const lastY = pad.top + plotH - ((vals[vals.length - 1] - min) / yRange) * plotH;
        ctx.beginPath();
        ctx.arc(lastX, lastY, 3, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.shadowColor = hexToRgba(color, 0.5);
        ctx.shadowBlur = 8;
        ctx.fill();
        ctx.shadowBlur = 0;
        ctx.shadowColor = 'transparent';
      };

      // Draw sub-lines first, composite on top
      drawLine('stakingBps', lineColors.staking, 1, false);
      drawLine('structuredBps', lineColors.structured, 1, false);
      drawLine('lendingBps', lineColors.lending, 1, false);
      drawLine('compositeBps', lineColors.composite, 2, true);
    },
    [sliced, lineColors],
  );

  // Summary stats
  const summary = useMemo(() => {
    if (sliced.length < 2) return null;
    const vals = sliced.map((r) => r.compositeBps);
    const min = Math.min(...vals);
    const max = Math.max(...vals);
    const avg = Math.round(vals.reduce((a, b) => a + b, 0) / vals.length);
    const delta = vals[vals.length - 1] - vals[0];
    const elapsed = Math.round((sliced[sliced.length - 1].timestampMs - sliced[0].timestampMs) / 1000);
    const elapsedLabel = elapsed >= 60 ? `${Math.floor(elapsed / 60)}m` : `${elapsed}s`;
    return { min, max, avg, delta, elapsedLabel };
  }, [sliced]);

  return (
    <div className="isfr-history-wrap dash-stagger" style={{ '--stagger-i': 2 } as React.CSSProperties}>
      <div
        className="isfr-history-chart"
        onMouseMove={handleMouseMove}
        onMouseLeave={handleMouseLeave}
      >
        <canvas ref={canvasRef} style={{ width: '100%', height: '100%', display: 'block' }} />
        <div ref={tooltipRef} className="isfr-chart-tooltip" style={{ opacity: 0 }} />
      </div>
      <div className="isfr-history-controls">
        <div className="isfr-history-range">
          {[60, 120, 256].map((n) => (
            <button
              key={n}
              className={`isfr-history-range-btn${range === n ? ' isfr-history-range-btn--active' : ''}`}
              onClick={() => setRange(n)}
            >
              {n}
            </button>
          ))}
        </div>
        <div className="isfr-history-legend">
          <span className="isfr-history-legend-item"><span className="isfr-history-legend-dot" style={{ background: lineColors.composite }} /> composite</span>
          <span className="isfr-history-legend-item"><span className="isfr-history-legend-dot" style={{ background: lineColors.lending }} /> lending</span>
          <span className="isfr-history-legend-item"><span className="isfr-history-legend-dot" style={{ background: lineColors.structured }} /> structured</span>
          <span className="isfr-history-legend-item"><span className="isfr-history-legend-dot" style={{ background: lineColors.staking }} /> staking</span>
        </div>
        {summary && (
          <span className="isfr-history-summary">
            {summary.delta >= 0 ? '\u25b2' : '\u25bc'} {summary.delta > 0 ? '+' : ''}{summary.delta} bps over {summary.elapsedLabel} &middot; min {summary.min} &middot; max {summary.max} &middot; avg {summary.avg}
          </span>
        )}
      </div>
    </div>
  );
}

/* ── Events Tab ──────────────────────────────────────────────── */

function EventsTab({ eventLog }: { eventLog: IsfrEventEntry[] }) {
  const [filter, setFilter] = useState<'all' | 'rate' | 'source' | 'keeper'>('all');
  const scrollRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const prevLenRef = useRef(eventLog.length);
  const [wsConnected, setWsConnected] = useState(false);
  const [wsEvents, setWsEvents] = useState<IsfrEventEntry[]>([]);

  // WS relay (lazy)
  useEffect(() => {
    let ws: WebSocket | null = null;
    let id = 0;
    try {
      ws = new WebSocket(`${WS_BASE}/api/workflow/ws`);
      ws.onopen = () => setWsConnected(true);
      ws.onclose = () => setWsConnected(false);
      ws.onerror = () => setWsConnected(false);
      ws.onmessage = (ev) => {
        try {
          const data = JSON.parse(ev.data);
          const msg = (data as { TopicMessage?: { topic: string; payload: unknown; timestamp_ms: number } }).TopicMessage ?? data;
          if (msg.topic && String(msg.topic).startsWith('isfr')) {
            id++;
            setWsEvents((prev) => [
              {
                ts: msg.timestamp_ms ?? Date.now(),
                type: 'rate' as const,
                message: `[WS] ${msg.topic}: ${JSON.stringify(msg.payload).slice(0, 120)}`,
              },
              ...prev.slice(0, 199),
            ]);
          }
        } catch {
          // ignore
        }
      };
    } catch {
      // ws not available
    }
    return () => { ws?.close(); setWsConnected(false); };
  }, []);

  // Merged + filtered log
  const merged = useMemo(() => {
    const all = [...eventLog, ...wsEvents].sort((a, b) => b.ts - a.ts).slice(0, 500);
    if (filter === 'all') return all;
    return all.filter((e) => e.type === filter);
  }, [eventLog, wsEvents, filter]);

  // Auto-scroll detection
  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    setAutoScroll(el.scrollTop <= 10);
  }, []);

  // Scroll to top when new events
  useEffect(() => {
    if (autoScroll && eventLog.length !== prevLenRef.current && scrollRef.current) {
      scrollRef.current.scrollTop = 0;
    }
    prevLenRef.current = eventLog.length;
  }, [eventLog.length, autoScroll]);

  return (
    <div className="isfr-events-wrap dash-stagger" style={{ '--stagger-i': 2 } as React.CSSProperties}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <div className="isfr-events-filter">
          {(['all', 'rate', 'source', 'keeper'] as const).map((f) => (
            <button
              key={f}
              className={`isfr-events-filter-btn${filter === f ? ' isfr-events-filter-btn--active' : ''}`}
              onClick={() => setFilter(f)}
            >
              {f}
            </button>
          ))}
        </div>
        <div className="dash-inline" style={{ gap: 8, fontSize: 'var(--text-xs)', fontFamily: 'var(--mono)', color: 'var(--text-ghost)' }}>
          <span
            className="dash-dot--5"
            style={{
              background: wsConnected ? 'var(--success)' : 'var(--text-ghost)',
              animation: wsConnected ? 'pulse-dot 2s ease-in-out infinite' : 'none',
            }}
          />
          WS {wsConnected ? 'connected' : 'disconnected'}
        </div>
      </div>
      <div
        ref={scrollRef}
        className="isfr-events-log"
        onScroll={handleScroll}
      >
        {!autoScroll && (
          <button
            className="isfr-events-jump"
            onClick={() => {
              if (scrollRef.current) scrollRef.current.scrollTop = 0;
              setAutoScroll(true);
            }}
          >
            &darr; Jump to latest
          </button>
        )}
        {merged.length === 0 && (
          <div className="isfr-events-empty">Waiting for events&hellip;</div>
        )}
        {merged.map((evt, i) => {
          const icon = evt.type === 'rate' ? '\u2713'
            : evt.type === 'source' ? '\u2192'
            : evt.type === 'keeper' ? '\u00B7'
            : '\u00B7';
          return (
            <div key={`${evt.ts}-${i}`} className="isfr-events-row">
              <span className={`isfr-events-icon isfr-events-icon--${evt.type}`}>
                {icon}
              </span>
              <span className="isfr-events-ts">{formatTs(evt.ts)}</span>
              <span className={`isfr-events-type isfr-events-type--${evt.type}`}>
                [{evt.type}]
              </span>
              <span className="isfr-events-msg">{stripAnsi(evt.message)}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/* ── Agents Tab ──────────────────────────────────────────────── */

const AGENT_EVENT_TYPES = [
  'agent_spawned',
  'agent_output',
  'agent_completed',
  'agent_failed',
  'agent_trace',
  'task_started',
  'task_completed',
  'task_failed',
  'gate_started',
  'gate_passed',
  'gate_failed',
  'gate_result',
  'feed_agent_online',
  'feed_agent_offline',
  'feed_tick',
] as const;

const MAX_AGENT_LOG_LINES = 500;

type AgentStatus = 'running' | 'completed' | 'failed';

interface TrackedAgent {
  id: string;
  name: string;
  role: string;
  model: string;
  status: AgentStatus;
  spawnedAt: number;
  lines: AgentLogLine[];
}

interface AgentLogLine {
  ts: number;
  type: 'output' | 'trace' | 'gate' | 'task' | 'lifecycle';
  level: 'info' | 'success' | 'error' | 'warning' | 'muted';
  text: string;
}

function classifyLogLevel(type: string, event: Record<string, unknown>): AgentLogLine['level'] {
  switch (type) {
    case 'agent_completed':
    case 'task_completed':
    case 'gate_passed':
      return 'success';
    case 'gate_result':
      return event.passed || event.ok ? 'success' : 'error';
    case 'agent_failed':
    case 'task_failed':
    case 'gate_failed':
      return 'error';
    case 'agent_trace':
      return 'muted';
    default:
      return 'info';
  }
}

function formatLogLine(type: string, event: Record<string, unknown>): string {
  switch (type) {
    case 'agent_output': {
      const text = event.text ?? event.output ?? event.content ?? '';
      return String(text).slice(0, 500);
    }
    case 'agent_trace': {
      const tool = event.tool_name ?? event.tool ?? '';
      const reasoning = event.reasoning ?? '';
      if (tool && reasoning) return `[${tool}] ${String(reasoning).slice(0, 300)}`;
      if (tool) return `tool_call: ${tool}`;
      if (reasoning) return String(reasoning).slice(0, 300);
      return JSON.stringify(event).slice(0, 200);
    }
    case 'agent_completed': {
      const tokens = event.total_tokens ?? event.tokens ?? '';
      const cost = event.cost ?? '';
      const parts = ['Agent completed'];
      if (tokens) parts.push(`tokens=${tokens}`);
      if (cost) parts.push(`cost=$${typeof cost === 'number' ? cost.toFixed(4) : cost}`);
      return parts.join(' ');
    }
    case 'agent_failed':
      return `Agent failed: ${event.error ?? event.message ?? 'unknown'}`;
    case 'task_started':
      return `Task ${event.task_id ?? event.task ?? ''} started${event.phase ? ` (${event.phase})` : ''}`;
    case 'task_completed':
      return `Task ${event.task_id ?? event.task ?? ''} completed [${event.status ?? 'ok'}]`;
    case 'task_failed':
      return `Task ${event.task_id ?? event.task ?? ''} failed: ${event.error ?? event.message ?? ''}`;
    case 'gate_started':
      return `Gate ${event.gate ?? event.name ?? ''} started`;
    case 'gate_passed':
      return `Gate ${event.gate ?? event.name ?? ''} passed`;
    case 'gate_failed':
      return `Gate ${event.gate ?? event.name ?? ''} failed: ${event.reason ?? event.message ?? ''}`;
    case 'gate_result':
      return `Gate ${event.gate ?? event.name ?? ''} ${(event.passed || event.ok) ? 'PASS' : 'FAIL'}`;
    default:
      return JSON.stringify(event).slice(0, 200);
  }
}

function lineTypeFromEvent(type: string): AgentLogLine['type'] {
  if (type.startsWith('agent_output')) return 'output';
  if (type.startsWith('agent_trace')) return 'trace';
  if (type.startsWith('gate_')) return 'gate';
  if (type.startsWith('task_')) return 'task';
  return 'lifecycle';
}

function AgentsTab() {
  const { get } = useLiveApi();
  const [agents, setAgents] = useState<Map<string, TrackedAgent>>(new Map());
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const agentsRef = useRef(agents);
  agentsRef.current = agents;

  // Seed from REST on mount so agents already running appear immediately.
  // Feed agents auto-populate from feed_tick SSE events within ~10s,
  // but we also fetch managed-agents for non-feed agents.
  useEffect(() => {
    (async () => {
      try {
        const res = await get<unknown>('/api/managed-agents');
        const agents = Array.isArray(res) ? res : [];
        setAgents((prev) => {
          const next = new Map(prev);
          for (const a of agents) {
            if (!a || typeof a !== 'object') continue;
            const rec = a as Record<string, unknown>;
            const id = String(rec.agent_id ?? '');
            if (!id || next.has(id)) continue;
            next.set(id, {
              id,
              name: String(rec.name ?? id),
              role: String(rec.role ?? 'agent'),
              model: String(rec.model ?? ''),
              status: 'running',
              spawnedAt: Date.now(),
              lines: [{ ts: Date.now(), type: 'lifecycle', level: 'info' as const, text: 'Agent discovered (already running)' }],
            });
          }
          return next;
        });
      } catch { /* ok - agents will populate from SSE feed_tick events */ }
    })();
  }, [get]);

  useContextEventSubscription(
    AGENT_EVENT_TYPES as unknown as string[],
    useCallback((event: unknown) => {
      if (!event || typeof event !== 'object') return;
      const ev = event as Record<string, unknown>;
      const type = typeof ev.type === 'string' ? ev.type : 'unknown';
      const agentId = String(ev.agentId ?? ev.agent_id ?? ev.agent_name ?? ev.name ?? '');
      if (!agentId) return;
      const ts = typeof ev.timestamp_ms === 'number' ? ev.timestamp_ms : Date.now();

      setAgents((prev) => {
        const next = new Map(prev);

        if (type === 'agent_spawned') {
          next.set(agentId, {
            id: agentId,
            name: String(ev.agent_name ?? ev.name ?? agentId),
            role: String(ev.role ?? ''),
            model: String(ev.model ?? ''),
            status: 'running',
            spawnedAt: ts,
            lines: [{ ts, type: 'lifecycle', level: 'info', text: `Agent spawned (model=${ev.model ?? '?'}, role=${ev.role ?? '?'})` }],
          });
          return next;
        }

        // Feed agents come online via feed_agent_online events.
        if (type === 'feed_agent_online') {
          const feedCount = typeof ev.feedCount === 'number' ? ev.feedCount : (typeof ev.feed_count === 'number' ? ev.feed_count : 0);
          next.set(agentId, {
            id: agentId,
            name: String(ev.name ?? agentId),
            role: 'feed-agent',
            model: 'native',
            status: 'running',
            spawnedAt: ts,
            lines: [{ ts, type: 'lifecycle', level: 'info', text: `Feed agent online (${feedCount} feeds)` }],
          });
          return next;
        }

        if (type === 'feed_agent_offline') {
          const existing = next.get(agentId);
          if (existing) {
            next.set(agentId, { ...existing, status: 'completed', lines: [...existing.lines, { ts, type: 'lifecycle', level: 'info', text: 'Feed agent offline' }] });
          }
          return next;
        }

        // Feed ticks → append as a log line on the agent; auto-create if unseen.
        if (type === 'feed_tick') {
          const topic = String(ev.topic ?? '');
          const payload = ev.payload as Record<string, unknown> | undefined;
          const preview = payload
            ? Object.entries(payload).slice(0, 3).map(([k, v]) => `${k}:${typeof v === 'number' ? (v as number).toFixed(0) : v}`).join(' ')
            : '';
          const line: AgentLogLine = { ts, type: 'output', level: 'info', text: `[${topic}] ${preview}` };
          const existing = next.get(agentId);
          if (existing) {
            next.set(agentId, { ...existing, status: 'running', lines: [...existing.lines, line].slice(-MAX_AGENT_LOG_LINES) });
          } else {
            // Auto-create agent from first feed_tick if we missed feed_agent_online
            next.set(agentId, {
              id: agentId,
              name: String(ev.name ?? agentId),
              role: 'feed-agent',
              model: 'native',
              status: 'running',
              spawnedAt: ts,
              lines: [line],
            });
          }
          return next;
        }

        const existing = next.get(agentId);
        if (!existing) {
          // Create agent entry on first non-spawn event
          const agent: TrackedAgent = {
            id: agentId,
            name: String(ev.agent_name ?? ev.name ?? agentId),
            role: '',
            model: '',
            status: 'running',
            spawnedAt: ts,
            lines: [],
          };
          next.set(agentId, agent);
        }

        const agent = next.get(agentId)!;
        const line: AgentLogLine = {
          ts,
          type: lineTypeFromEvent(type),
          level: classifyLogLevel(type, ev),
          text: formatLogLine(type, ev),
        };
        const newLines = [...agent.lines, line].slice(-MAX_AGENT_LOG_LINES);

        let status = agent.status;
        if (type === 'agent_completed') status = 'completed';
        if (type === 'agent_failed') status = 'failed';

        next.set(agentId, { ...agent, status, lines: newLines });
        return next;
      });
    }, []),
  );

  const agentList = useMemo(() =>
    Array.from(agents.values()).sort((a, b) => b.spawnedAt - a.spawnedAt),
    [agents],
  );

  const expanded = expandedId ? agents.get(expandedId) ?? null : null;

  return (
    <div className="isfr-agents-wrap dash-stagger" style={{ '--stagger-i': 2 } as React.CSSProperties}>
      <div className="isfr-agents-header">
        <div className="isfr-agents-meta">
          <span className="isfr-agents-count">
            {agentList.filter((a) => a.status === 'running').length} running
          </span>
          <span className="isfr-agents-count">{agentList.length} total</span>
        </div>
      </div>

      {agentList.length === 0 ? (
        <div className="isfr-events-empty">Waiting for agent activity&hellip;</div>
      ) : expanded ? (
        /* ── Expanded single-agent view ─── */
        <AgentLogPanel agent={expanded} onBack={() => setExpandedId(null)} />
      ) : (
        /* ── Grid of agent cards ─── */
        <div className="isfr-agents-grid">
          {agentList.map((agent, idx) => (
            <AgentCard key={agent.id} agent={agent} index={idx} onExpand={() => setExpandedId(agent.id)} />
          ))}
        </div>
      )}
    </div>
  );
}

/* ── Agent Card (summary with live mini-log) ─── */

function AgentCard({ agent, index, onExpand }: { agent: TrackedAgent; index: number; onExpand: () => void }) {
  const bottomRef = useRef<HTMLDivElement>(null);
  const recentLines = useMemo(() => agent.lines.slice(-20), [agent.lines]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
  }, [recentLines.length]);

  const statusColor =
    agent.status === 'running' ? 'var(--success)' :
    agent.status === 'completed' ? 'var(--text-dim)' :
    'var(--rose-bright)';

  return (
    <div
      className={`agent-card${agent.status === 'running' ? ' agent-card--running' : ''}`}
      style={{ '--card-i': index } as React.CSSProperties}
      onClick={onExpand}
    >
      <div className="agent-card-header">
        <span className="agent-card-dot" style={{ background: statusColor }} />
        <span className="agent-card-name">{agent.name}</span>
        <span className="agent-card-status">{agent.status}</span>
      </div>
      {(agent.role || agent.model) && (
        <div className="agent-card-meta">
          {agent.role && <span className="agent-card-tag">{agent.role}</span>}
          {agent.model && <span className="agent-card-tag">{agent.model}</span>}
        </div>
      )}
      <div className="agent-card-log">
        {recentLines.map((line, i) => (
          <div key={`${line.ts}-${i}`} className={`agent-card-line agent-card-line--${line.level}`}>
            {line.text}
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
      <div className="agent-card-footer">
        <span className="agent-card-lines-count">{agent.lines.length} lines</span>
        <span className="agent-card-expand-hint">click to expand</span>
      </div>
    </div>
  );
}

/* ── Expanded Agent Log Panel ─── */

const LOG_LEVEL_ICONS: Record<AgentLogLine['level'], string> = {
  success: '\u2713',
  error:   '\u2717',
  warning: '\u21BB',
  info:    '\u2192',
  muted:   '\u00B7',
};

function AgentLogPanel({ agent, onBack }: { agent: TrackedAgent; onBack: () => void }) {
  const bottomRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [typeFilter, setTypeFilter] = useState<AgentLogLine['type'] | 'all'>('all');

  const filtered = useMemo(() => {
    if (typeFilter === 'all') return agent.lines;
    return agent.lines.filter((l) => l.type === typeFilter);
  }, [agent.lines, typeFilter]);

  useEffect(() => {
    if (autoScroll) {
      bottomRef.current?.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    }
  }, [filtered.length, autoScroll]);

  const handleScroll = useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    setAutoScroll(nearBottom);
  }, []);

  const statusColor =
    agent.status === 'running' ? 'var(--success)' :
    agent.status === 'completed' ? 'var(--text-dim)' :
    'var(--rose-bright)';

  const TYPES: Array<AgentLogLine['type'] | 'all'> = ['all', 'output', 'trace', 'gate', 'task', 'lifecycle'];

  return (
    <div className="agent-panel">
      <div className="agent-panel-header">
        <button className="agent-panel-back" onClick={onBack}>&larr; All Agents</button>
        <span className="agent-card-dot" style={{ background: statusColor }} />
        <span className="agent-panel-name">{agent.name}</span>
        {agent.role && <span className="agent-card-tag">{agent.role}</span>}
        {agent.model && <span className="agent-card-tag">{agent.model}</span>}
        <span className="agent-panel-count">{filtered.length} lines</span>
      </div>
      <div className="agent-panel-filters">
        {TYPES.map((t) => (
          <button
            key={t}
            className={`isfr-events-filter-btn${typeFilter === t ? ' isfr-events-filter-btn--active' : ''}`}
            onClick={() => setTypeFilter(t)}
          >
            {t}
          </button>
        ))}
      </div>
      <div ref={containerRef} className="agent-panel-log" onScroll={handleScroll}>
        {!autoScroll && (
          <button
            className="isfr-events-jump"
            style={{ position: 'sticky', top: 0, zIndex: 2 }}
            onClick={() => {
              bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
              setAutoScroll(true);
            }}
          >
            &darr; Jump to latest
          </button>
        )}
        {filtered.length === 0 && (
          <div className="isfr-events-empty">No {typeFilter} output yet&hellip;</div>
        )}
        {filtered.map((line, i) => (
          <div key={`${line.ts}-${i}`} className={`agent-panel-line agent-panel-line--${line.level}`}>
            <span className="agent-panel-line-icon">{LOG_LEVEL_ICONS[line.level]}</span>
            <span className="agent-panel-line-ts">{formatTs(line.ts)}</span>
            <span className={`agent-panel-line-type agent-panel-line-type--${line.type}`}>[{line.type}]</span>
            <span className="agent-panel-line-text">{line.text}</span>
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}

/* ── On-Chain Tab ────────────────────────────────────────────── */

function OnChainTab() {
  return (
    <div className="isfr-onchain-placeholder">
      No oracle deployed. Set <code>auto_deploy_contracts = true</code> in roko.toml
    </div>
  );
}

/* ── Helpers ─────────────────────────────────────────────────── */

function deltaStr(arr: number[]): string {
  if (arr.length < 2) return '\u2014';
  const d = arr[arr.length - 1] - arr[arr.length - 2];
  if (d === 0) return '\u2014';
  return d > 0 ? `+${d}` : `${d}`;
}

function formatTs(ms: number): string {
  const d = new Date(ms);
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  const ss = String(d.getSeconds()).padStart(2, '0');
  const mmm = String(d.getMilliseconds()).padStart(3, '0');
  return `${hh}:${mm}:${ss}.${mmm}`;
}

function getCssVar(name: string): string {
  return (
    getComputedStyle(document.documentElement).getPropertyValue(name).trim() ||
    '#e8b5ce'
  );
}

function hexToRgba(color: string, alpha: number): string {
  if (color.startsWith('#')) {
    const hex = color.slice(1);
    const r = parseInt(hex.slice(0, 2), 16);
    const g = parseInt(hex.slice(2, 4), 16);
    const b = parseInt(hex.slice(4, 6), 16);
    return `rgba(${r},${g},${b},${alpha})`;
  }
  const match = color.match(/(\d+),\s*(\d+),\s*(\d+)/);
  if (match) return `rgba(${match[1]},${match[2]},${match[3]},${alpha})`;
  return `rgba(200,150,180,${alpha})`;
}

/** Resolve a CSS var reference to an actual color value. */
function useResolvedColor(cssColor: string): string {
  const [resolved, setResolved] = useState(cssColor);
  useEffect(() => {
    if (cssColor.startsWith('var(')) {
      const name = cssColor.replace(/^var\(/, '').replace(/\)$/, '');
      setResolved(getCssVar(name));
    } else {
      setResolved(cssColor);
    }
  }, [cssColor]);
  return resolved;
}
