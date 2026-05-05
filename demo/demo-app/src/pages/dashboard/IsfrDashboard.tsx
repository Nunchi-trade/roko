import { useState, useEffect, useCallback, useRef } from 'react';
import { useLiveApi } from '../../hooks/useLiveApi';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
import { useCountUp } from '../../hooks/useCountUp';
import { useCanvasSetup } from '../../hooks/useCanvasSetup';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import {
  type IsfrRate,
  type IsfrSource,
  type IsfrStatus,
  formatBps,
  formatPercent,
  formatConfidence,
} from '../../lib/isfr-api';
import { WS_BASE } from '../../lib/serve-url';
import '../dashboard/dashboard.css';
import './IsfrDashboard.css';

/* ── Phosphor decay hook ─────────────────────────────────── */

function usePhosphorDecay(value: number): boolean {
  const prevRef = useRef(value);
  const [flashing, setFlashing] = useState(false);
  useEffect(() => {
    if (prevRef.current !== value && value !== 0) {
      setFlashing(true);
      const id = setTimeout(() => setFlashing(false), 300);
      prevRef.current = value;
      return () => clearTimeout(id);
    }
    prevRef.current = value;
  }, [value]);
  return flashing;
}

/* ── Types ───────────────────────────────────────────────── */

interface RelayMessage {
  id: number;
  topic: string;
  msg_type: string;
  payload: unknown;
  publisher_id: string | null;
  seq: number;
  timestamp_ms: number;
}

type TabId = 'overview' | 'sources' | 'history' | 'relay';

/* ── Constants ───────────────────────────────────────────── */

const MAX_HISTORY = 60;
const MAX_RELAY_MESSAGES = 200;

const HEALTH_COLORS: Record<string, string> = {
  healthy: 'var(--success)',
  degraded: 'var(--warning)',
  stale: 'var(--text-dim)',
  failed: 'var(--rose-bright)',
  down: 'var(--rose-bright)',
};

/* ── Component ───────────────────────────────────────────── */

export default function IsfrDashboard() {
  const { get } = useLiveApi();
  const [tab, setTab] = useState<TabId>('overview');
  const [status, setStatus] = useState<IsfrStatus | null>(null);
  const [currentRate, setCurrentRate] = useState<IsfrRate | null>(null);
  const [sources, setSources] = useState<IsfrSource[]>([]);
  const [history, setHistory] = useState<IsfrRate[]>([]);
  const [relayMessages, setRelayMessages] = useState<RelayMessage[]>([]);
  const [relayConnected, setRelayConnected] = useState(false);
  const [initialLoading, setInitialLoading] = useState(true);

  const fetchAll = useCallback(async () => {
    const [s, r, src, h] = await Promise.all([
      get<IsfrStatus>('/api/isfr/status'),
      get<IsfrRate | null>('/api/isfr/current'),
      get<IsfrSource[]>('/api/isfr/sources'),
      get<IsfrRate[]>('/api/isfr/history?limit=' + MAX_HISTORY),
    ]);
    setStatus(s);
    setCurrentRate(r);
    setSources(src);
    setHistory(h);
  }, [get]);

  // Initial fetch + 30s fallback poll
  useEffect(() => {
    fetchAll().finally(() => setInitialLoading(false));
    const id = setInterval(fetchAll, 30_000);
    return () => clearInterval(id);
  }, [fetchAll]);

  // SSE-triggered refetch (debounced 2s)
  const debouncedRefetch = useDebouncedRefetch(fetchAll, 2000);
  useContextEventSubscription(
    ['isfr_rate_computed', 'isfr_source_health'],
    debouncedRefetch,
  );

  // WebSocket for relay tab — lazy, only when relay tab is active
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
          const msg =
            (data as { TopicMessage?: RelayMessage }).TopicMessage ??
            (data as RelayMessage);
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
      // WebSocket creation failed (e.g. relay not running)
    }

    return () => {
      ws?.close();
      setRelayConnected(false);
    };
  }, [tab]);

  /* Derived values */
  const compositeBps = currentRate?.composite_bps ?? 0;
  const confidenceBps = currentRate?.confidence_bps ?? 0;
  const keeperRunning = status?.keeper_running ?? false;
  const sourcesCount = status?.sources_count ?? 0;
  const epoch = status?.current_epoch ?? 0;
  const confidencePct = confidenceBps / 100; // 0–100 range

  /* Animated counters */
  const animComposite = useCountUp(compositeBps, 900);
  const animConfidence = useCountUp(confidencePct, 800);

  /* Phosphor flash */
  const compositeFlash = usePhosphorDecay(compositeBps);
  const confidenceFlash = usePhosphorDecay(confidenceBps);

  /* Sparkline data */
  const sparklineData = history.map((r) => r.composite_bps);

  /* Source health summary */
  const healthyCt = sources.filter((s) => s.health === 'healthy').length;

  const TABS: { id: TabId; label: string }[] = [
    { id: 'overview', label: 'Overview' },
    { id: 'sources', label: 'Sources' },
    { id: 'history', label: 'History' },
    { id: 'relay', label: 'Relay' },
  ];

  if (initialLoading) {
    return (
      <div className="dash-page progressive-reveal cd-skeleton-layout">
        <div className="skeleton cd-skeleton-hero" />
        <div className="cd-skeleton-grid">
          <div className="skeleton-card skeleton" />
          <div className="skeleton-card skeleton" />
          <div className="skeleton-card skeleton" />
        </div>
        <div className="skeleton-chart skeleton" />
      </div>
    );
  }

  return (
    <div className="dash-page">
      {/* STATUS BANNER — always visible */}
      <div className="dash-stagger gradient-border-subtle" style={{ '--stagger-i': 0 } as React.CSSProperties}>
        <Mosaic columns={5}>
          <MosaicCell
            label="KEEPER"
            value={
              <span className="dash-inline">
                <span
                  className="dash-dot"
                  style={{
                    background: keeperRunning ? 'var(--success)' : 'var(--rose-bright)',
                    boxShadow: keeperRunning ? 'var(--glow-success)' : 'var(--glow-error)',
                    animation: keeperRunning ? 'pulse-dot 2s ease-in-out infinite' : 'none',
                  }}
                />
                <span className="dash-mono-label">{keeperRunning ? 'Running' : 'Stopped'}</span>
              </span>
            }
            color="success"
            sub={`epoch ${epoch}`}
          />
          <MosaicCell
            label="COMPOSITE RATE"
            value={
              <span className={compositeFlash ? 'phosphor-flash' : ''}>
                {formatBps(Math.round(animComposite))}
              </span>
            }
            color="rose"
            mono
            sub={formatPercent(compositeBps)}
          />
          <MosaicCell
            label="CONFIDENCE"
            value={
              <span className={confidenceFlash ? 'phosphor-flash' : ''}>
                {animConfidence.toFixed(1)}%
              </span>
            }
            color="bone"
            mono
            sub={`${confidenceBps} / 10000 bps`}
          />
          <MosaicCell
            label="SOURCES"
            value={`${healthyCt}/${sourcesCount}`}
            color="dream"
            sub={`${healthyCt} healthy`}
          />
          <MosaicCell
            label="EPOCH"
            value={String(epoch)}
            color="warning"
            mono
            sub={status?.epoch_duration_secs ? `${status.epoch_duration_secs}s duration` : '\u2014'}
          />
        </Mosaic>
      </div>

      {/* TAB BAR */}
      <div className="isfr-tab-bar">
        {TABS.map((t) => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            className={`isfr-tab-btn${tab === t.id ? ' isfr-tab-btn--active' : ''}`}
          >
            {t.label}
          </button>
        ))}
      </div>

      {/* TAB CONTENT — if/else-if chain (not switch) */}
      {tab === 'overview' && (
        <OverviewTab
          currentRate={currentRate}
          compositeBps={compositeBps}
          history={history}
          status={status}
        />
      )}
      {tab === 'overview' ? null : tab === 'sources' && (
        <SourcesTab sources={sources} sourcesCount={sourcesCount} healthyCt={healthyCt} />
      )}
      {tab === 'overview' ? null : tab === 'sources' ? null : tab === 'history' && (
        <HistoryTab sparklineData={sparklineData} history={history} />
      )}
      {tab === 'overview' ? null : tab === 'sources' ? null : tab === 'history' ? null : tab === 'relay' && (
        <RelayTab connected={relayConnected} messages={relayMessages} />
      )}
    </div>
  );
}

/* ── Overview Tab ────────────────────────────────────────── */

function OverviewTab({
  currentRate,
  compositeBps,
  history,
  status,
}: {
  currentRate: IsfrRate | null;
  compositeBps: number;
  history: IsfrRate[];
  status: IsfrStatus | null;
}) {
  const confidenceBps = currentRate?.confidence_bps ?? 0;
  const confidencePct = confidenceBps / 100;

  return (
    <>
      {/* MIDDLE ROW: Rate Breakdown + Confidence */}
      <div className="dash-flex-row">
        {/* Left: Per-class rate breakdown */}
        <div className="dash-flex-1 dash-stagger" style={{ '--stagger-i': 1 } as React.CSSProperties}>
          <Pane
            title="RATE BREAKDOWN"
            badge={<span className="dash-badge--glow">{formatBps(compositeBps)}</span>}
          >
            <Mosaic columns={4}>
              <MosaicCell
                label="LENDING"
                value={formatBps(currentRate?.lending_bps ?? null)}
                color="rose"
                mono
                sparkline={history.map((r) => r.lending_bps)}
              />
              <MosaicCell
                label="STRUCTURED"
                value={formatBps(currentRate?.structured_bps ?? null)}
                color="bone"
                mono
                sparkline={history.map((r) => r.structured_bps)}
              />
              <MosaicCell
                label="FUNDING"
                value={formatBps(currentRate?.funding_bps ?? null)}
                color="dream"
                mono
                sparkline={history.map((r) => r.funding_bps)}
              />
              <MosaicCell
                label="STAKING"
                value={formatBps(currentRate?.staking_bps ?? null)}
                color="warning"
                mono
                sparkline={history.map((r) => r.staking_bps)}
              />
            </Mosaic>
          </Pane>
        </div>

        {/* Right: Confidence meter */}
        <div className="dash-flex-1 dash-stagger" style={{ '--stagger-i': 2 } as React.CSSProperties}>
          <Pane title="CONFIDENCE" badge={<span className="dash-badge">{formatConfidence(confidenceBps)}</span>}>
            <div className="isfr-confidence-meter">
              <div className="isfr-confidence-track">
                <div
                  className="isfr-confidence-fill"
                  style={{
                    width: `${confidencePct}%`,
                    background:
                      confidencePct >= 80
                        ? 'var(--success)'
                        : confidencePct >= 50
                          ? 'var(--warning)'
                          : 'var(--rose-bright)',
                  }}
                />
              </div>
              <div className="isfr-confidence-labels">
                <span>0%</span>
                <span>50%</span>
                <span>100%</span>
              </div>
            </div>
            <div className="isfr-confidence-sub">
              <span className="dash-label-sm">Poll interval</span>
              <span className="isfr-meta-value">{status?.poll_interval_secs ?? '\u2014'}s</span>
            </div>
          </Pane>
        </div>
      </div>
    </>
  );
}

/* ── Sources Tab ─────────────────────────────────────────── */

function SourcesTab({
  sources,
  sourcesCount,
  healthyCt,
}: {
  sources: IsfrSource[];
  sourcesCount: number;
  healthyCt: number;
}) {
  return (
    <div className="dash-stagger" style={{ '--stagger-i': 1 } as React.CSSProperties}>
      <Pane
        title="SOURCE HEALTH"
        badge={
          <span className="dash-badge">
            {healthyCt}/{sourcesCount} healthy
          </span>
        }
      >
        <div className="isfr-source-table">
          <div className="isfr-source-header">
            <span>Source</span>
            <span>Class</span>
            <span>Weight</span>
            <span>Last Rate</span>
            <span>Health</span>
          </div>
          {sources.length === 0 && (
            <div className="isfr-source-empty">No sources registered</div>
          )}
          {sources.map((src) => (
            <div key={src.id} className="isfr-source-row">
              <span className="isfr-source-name">{src.name}</span>
              <span className="isfr-source-class">{src.class}</span>
              <span className="isfr-source-weight">{(src.weight * 100).toFixed(0)}%</span>
              <span className="isfr-source-rate mono">{formatBps(src.last_rate_bps)}</span>
              <span className="isfr-source-health">
                <span
                  className="dash-dot--7"
                  style={{
                    background: HEALTH_COLORS[src.health] ?? 'var(--text-dim)',
                    boxShadow:
                      src.health === 'healthy' ? 'var(--glow-success)' : 'none',
                    animation:
                      src.health === 'healthy'
                        ? 'pulse-dot 2s ease-in-out infinite'
                        : 'none',
                  }}
                />
                <span>{src.health}</span>
              </span>
            </div>
          ))}
        </div>
      </Pane>
    </div>
  );
}

/* ── History Tab ─────────────────────────────────────────── */

function HistoryTab({
  sparklineData,
  history,
}: {
  sparklineData: number[];
  history: IsfrRate[];
}) {
  return (
    <div className="dash-stagger" style={{ '--stagger-i': 1 } as React.CSSProperties}>
      <Pane
        title="RATE HISTORY"
        badge={<span className="dash-badge">{history.length} readings</span>}
      >
        <div className="dash-chart-enter">
          <IsfrRateChart data={sparklineData} height={200} />
        </div>
      </Pane>
    </div>
  );
}

/* ── Relay Tab ───────────────────────────────────────────── */

function RelayTab({
  connected,
  messages,
}: {
  connected: boolean;
  messages: RelayMessage[];
}) {
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = 0;
    }
  }, [messages.length]);

  return (
    <div className="dash-stagger" style={{ '--stagger-i': 1 } as React.CSSProperties}>
      <Pane
        title="RELAY MESSAGES"
        badge={
          <span className="dash-inline">
            <span
              className="dash-dot--7"
              style={{
                background: connected ? 'var(--success)' : 'var(--rose-bright)',
                animation: connected ? 'pulse-dot 2s ease-in-out infinite' : 'none',
              }}
            />
            <span className="dash-mono-label">{connected ? 'Connected' : 'Disconnected'}</span>
          </span>
        }
      >
        <div className="isfr-relay-info">
          Showing ISFR topic messages from relay WebSocket.
        </div>
        <div ref={scrollRef} className="isfr-relay-log">
          {messages.length === 0 && (
            <p className="isfr-relay-empty">
              Waiting for ISFR topic messages&hellip;
            </p>
          )}
          {messages.map((msg) => (
            <div key={msg.id} className="isfr-relay-row">
              <div className="isfr-relay-row-header">
                <span className="isfr-relay-topic">{msg.topic}</span>
                <span className="isfr-relay-meta">
                  seq:{msg.seq} &middot; {new Date(msg.timestamp_ms).toLocaleTimeString()}
                </span>
              </div>
              <div className="isfr-relay-body">
                {msg.msg_type}: {JSON.stringify(msg.payload).slice(0, 200)}
              </div>
            </div>
          ))}
        </div>
      </Pane>
    </div>
  );
}

/* ── ISFR Rate Chart (canvas sparkline) ──────────────────── */

interface IsfrRateChartProps {
  data: number[];
  height?: number;
}

function IsfrRateChart({ data, height = 140 }: IsfrRateChartProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useCanvasSetup(
    canvasRef,
    (ctx, w, h) => {
      const pad = { top: 16, right: 12, bottom: 12, left: 44 };
      const plotW = w - pad.left - pad.right;
      const plotH = h - pad.top - pad.bottom;

      ctx.clearRect(0, 0, w, h);

      if (data.length < 2) {
        ctx.fillStyle = getCssVar('--text-ghost');
        ctx.font = '10px "JetBrains Mono", monospace';
        ctx.fillText('Waiting for rate data\u2026', pad.left, pad.top + 20);
        return;
      }

      const min = Math.min(...data) - 10;
      const max = Math.max(...data) + 10;
      const range = max - min || 1;

      // Y-axis grid lines
      ctx.strokeStyle = 'rgba(255,255,255,0.05)';
      ctx.lineWidth = 1;
      ctx.fillStyle = getCssVar('--text-ghost');
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'right';
      for (let i = 0; i <= 3; i++) {
        const yy = pad.top + plotH * (1 - i / 3);
        ctx.beginPath();
        ctx.moveTo(pad.left, yy);
        ctx.lineTo(pad.left + plotW, yy);
        ctx.stroke();
        const label = Math.round(min + (range * i) / 3);
        ctx.fillText(`${label}`, pad.left - 6, yy + 3);
      }

      // Gradient fill under line
      const lineColor = getCssVar('--rose-glow');
      ctx.beginPath();
      for (let i = 0; i < data.length; i++) {
        const x = pad.left + (i / (data.length - 1)) * plotW;
        const y = pad.top + plotH - ((data[i] - min) / range) * plotH;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.lineTo(pad.left + plotW, pad.top + plotH);
      ctx.lineTo(pad.left, pad.top + plotH);
      ctx.closePath();
      ctx.fillStyle = hexToRgba(lineColor, 0.08);
      ctx.fill();

      // Main line
      ctx.beginPath();
      ctx.strokeStyle = lineColor;
      ctx.lineWidth = 2;
      ctx.lineJoin = 'round';
      ctx.lineCap = 'round';
      for (let i = 0; i < data.length; i++) {
        const x = pad.left + (i / (data.length - 1)) * plotW;
        const y = pad.top + plotH - ((data[i] - min) / range) * plotH;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.stroke();

      // Endpoint dot with glow
      const lastX = pad.left + plotW;
      const lastY =
        pad.top + plotH - ((data[data.length - 1] - min) / range) * plotH;
      ctx.beginPath();
      ctx.arc(lastX, lastY, 3, 0, Math.PI * 2);
      ctx.fillStyle = lineColor;
      ctx.shadowColor = hexToRgba(lineColor, 0.45);
      ctx.shadowBlur = 8;
      ctx.fill();
      ctx.shadowBlur = 0;
      ctx.shadowColor = 'transparent';

      // Latest value label
      ctx.fillStyle = lineColor;
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'right';
      ctx.fillText(`${data[data.length - 1]} bps`, w - pad.right, pad.top - 2);
    },
    [data],
  );

  return (
    <div className="chart-container" style={{ height }}>
      <canvas
        ref={canvasRef}
        className="chart-canvas"
        role="img"
        aria-label="ISFR composite rate history"
      />
    </div>
  );
}

/* ── Helpers ─────────────────────────────────────────────── */

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
