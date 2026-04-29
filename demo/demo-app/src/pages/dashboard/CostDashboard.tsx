import { useState, useEffect, useRef, useCallback } from 'react';
import { useLiveApi } from '../../hooks/useLiveApi';
import { fmtUptime } from '../../lib/format';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import CostChart from '../../components/Charts/CostChart';
import CFactorSparkline from '../../components/Charts/CFactorSparkline';
import BarChart from '../../components/Charts/BarChart';
import { ThresholdGaugeRow } from '../../components/ThresholdGauge';
import type { AdaptiveThresholdsResponse } from '../../components/ThresholdGauge';
import './dashboard.css';

/* ── useCountUp: animates from 0 → target on first mount ─── */

function useCountUp(target: number, duration = 900): number {
  const [val, setVal] = useState(0);
  const prevTarget = useRef<number | null>(null);
  const valRef = useRef(0);

  useEffect(() => {
    if (prevTarget.current === target) return;
    prevTarget.current = target;
    const start = performance.now();
    const from = valRef.current;
    let frame = 0;
    const tick = (now: number) => {
      const t = Math.min((now - start) / duration, 1);
      const eased = 1 - Math.pow(1 - t, 3);
      const next = from + (target - from) * eased;
      valRef.current = next;
      setVal(next);
      if (t < 1) frame = requestAnimationFrame(tick);
      else {
        valRef.current = target;
        setVal(target);
      }
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [target, duration]);
  return val;
}

/* ── API shapes ──────────────────────────────────────────── */

interface HealthResponse {
  status: string;
  uptime_secs: number;
  version: string;
  active_plans: number;
  active_agents: number;
  providers?: { healthy: number; total: number };
  statehub?: {
    snapshot?: {
      cost_usd_total: number;
      episodes_total: number;
      gates_passed: number;
      gates_failed: number;
    };
  };
}

interface EfficiencyResponse {
  total_cost: number;
  cost_per_task: number;
  tasks: { task_id: string; cost_usd: number; tokens: number; duration_ms: number }[];
}

interface CFactorResponse {
  composite: { overall: number; episode_count: number };
  sub_metrics: Record<string, number>;
}

interface CFactorTrendResponse {
  trend: { start: string; samples: number; avg: number; p50: number; p95: number }[];
  woolley?: Record<string, number[]>;
}

interface RouterResponse {
  model_slugs?: string[];
  role_table?: Record<string, string>;
  confidence_stats?: Record<string, { successes: number; trials: number; total_cost_usd: number }>;
  total_observations?: number;
}

type ProviderStatus = 'healthy' | 'degraded' | 'unhealthy';

interface Provider {
  name: string;
  status: ProviderStatus;
  models: string[];
  success_rate: number;
  avg_latency_ms: number;
  p95_latency_ms: number;
  cost_per_1k_tokens: number;
  total_requests: number;
  errors_24h: number;
  last_error: string;
}

interface ProviderHealthResponse {
  providers: Provider[];
}

const STATUS_DOT_STYLES: Record<ProviderStatus, { bg: string; glow: string; anim: string }> = {
  healthy: { bg: 'var(--success)', glow: '0 0 6px rgba(122,138,120,.6)', anim: 'pulse-dot 2s ease-in-out infinite' },
  degraded: { bg: 'var(--warning)', glow: '0 0 6px rgba(216,168,120,.6)', anim: 'pulse-dot 1.5s ease-in-out infinite' },
  unhealthy: { bg: 'var(--rose-bright)', glow: '0 0 6px rgba(204,144,168,.6)', anim: 'none' },
};

/* ── Metric bar config ───────────────────────────────────── */

const METRICS: { key: string; label: string; color: string }[] = [
  { key: 'gate_pass_rate',  label: 'Gate Pass Rate',  color: 'var(--success)' },
  { key: 'cost_efficiency', label: 'Cost Efficiency',  color: 'var(--bone)' },
  { key: 'speed',           label: 'Speed',            color: 'var(--dream-bright)' },
  { key: 'reuse_rate',      label: 'Reuse Rate',       color: 'var(--rose-bright)' },
  { key: 'learning_rate',   label: 'Learning Rate',    color: 'var(--warning)' },
];

/* ── Component ───────────────────────────────────────────── */

export default function CostDashboard() {
  const { get } = useLiveApi();
  const [health, setHealth] = useState<HealthResponse | null>(null);
  const [efficiency, setEfficiency] = useState<EfficiencyResponse | null>(null);
  const [cfactor, setCfactor] = useState<CFactorResponse | null>(null);
  const [cfactorTrend, setCfactorTrend] = useState<CFactorTrendResponse | null>(null);
  const [router, setRouter] = useState<RouterResponse | null>(null);
  const [providerHealth, setProviderHealth] = useState<ProviderHealthResponse | null>(null);
  const [thresholds, setThresholds] = useState<AdaptiveThresholdsResponse | null>(null);

  const fetchAll = useCallback(async () => {
    const [h, e, c, t, r, ph, th] = await Promise.all([
      get<HealthResponse>('/api/health'),
      get<EfficiencyResponse>('/api/learn/efficiency'),
      get<CFactorResponse>('/api/metrics/c_factor'),
      get<CFactorTrendResponse>('/api/c-factor/trend?window=24h'),
      get<RouterResponse>('/api/learn/cascade-router'),
      get<ProviderHealthResponse>('/api/learn/provider-outcomes'),
      get<AdaptiveThresholdsResponse>('/api/learn/adaptive-thresholds'),
    ]);
    setHealth(h);
    setEfficiency(e);
    setCfactor(c);
    setCfactorTrend(t);
    setRouter(r);
    setProviderHealth(ph);
    setThresholds(th);
  }, [get]);

  // Initial fetch on mount
  useEffect(() => { fetchAll(); }, [fetchAll]);

  // SSE-triggered refetch
  const debouncedRefetch = useDebouncedRefetch(fetchAll, 2000);
  useContextEventSubscription(
    ['efficiency_event', 'gate_result', 'episode', 'inference_completed'],
    debouncedRefetch,
  );

  /* Derived */
  const snap = health?.statehub?.snapshot;
  const totalCost = efficiency?.total_cost ?? snap?.cost_usd_total ?? 0;
  const episodes = snap?.episodes_total ?? 0;
  const isOnline = health?.status === 'ok';
  const composite = cfactor?.composite?.overall ?? 0;
  const subMetrics = cfactor?.sub_metrics ?? {};

  /* Animated counters */
  const animComposite = useCountUp(composite, 1100);
  const animCost = useCountUp(totalCost, 900);
  const animEpisodes = useCountUp(episodes, 800);

  /* Cost chart points from tasks */
  const costPoints = (efficiency?.tasks ?? []).map((t, i) => ({
    label: `T${i + 1}`,
    value: t.cost_usd,
  }));
  const providers = providerHealth?.providers ?? [];
  const healthyProviders = providers.filter((p) => p.status === 'healthy').length;

  /* Router bar data: derive model distribution from role_table */
  const routerBars = (() => {
    const rt = router?.role_table;
    if (!rt || Object.keys(rt).length === 0) return [];
    const modelCounts: Record<string, number> = {};
    for (const model of Object.values(rt)) {
      modelCounts[model] = (modelCounts[model] ?? 0) + 1;
    }
    const totalRoles = Object.values(modelCounts).reduce((a, b) => a + b, 0);
    const colors = ['var(--success)', 'var(--rose-bright)', 'var(--bone)', 'var(--dream-bright)'];
    return Object.entries(modelCounts)
      .sort(([, a], [, b]) => b - a)
      .map(([model, count], i) => ({
        label: model.replace(/^claude-/, ''),
        value: (count / totalRoles) * 100,
        color: colors[i % colors.length],
      }));
  })();

  /* Derive current model: most-assigned model in role_table */
  const currentModel = (() => {
    const rt = router?.role_table;
    if (!rt || Object.keys(rt).length === 0) return '—';
    const modelCounts: Record<string, number> = {};
    for (const model of Object.values(rt)) {
      modelCounts[model] = (modelCounts[model] ?? 0) + 1;
    }
    return Object.entries(modelCounts).sort(([, a], [, b]) => b - a)[0]?.[0] ?? '—';
  })();

  const gatesPassed = snap?.gates_passed ?? 0;
  const gatesFailed = snap?.gates_failed ?? 0;
  const gateTotal = gatesPassed + gatesFailed;
  const gatePassRate = gateTotal > 0 ? `${((gatesPassed / gateTotal) * 100).toFixed(1)}%` : '—';

  return (
    <div className="dash-page">
      {/* TOP MOSAIC: 6 stats */}
      <Mosaic columns={6}>
        <MosaicCell
          label="STATUS"
          value={
            <span className="dash-inline">
              <span
                className="dash-dot"
                style={{
                  background: isOnline ? 'var(--success)' : 'var(--rose-bright)',
                  boxShadow: isOnline ? '0 0 6px rgba(122,138,120,.6)' : '0 0 6px rgba(204,144,168,.5)',
                  animation: isOnline ? 'pulse-dot 2s ease-in-out infinite' : 'none',
                }}
              />
              <span className="dash-mono-label">{isOnline ? 'Online' : 'Offline'}</span>
            </span>
          }
          color="success"
          sub={health?.providers ? `${health.providers.healthy}/${health.providers.total} providers` : '0/0 providers'}
        />
        <MosaicCell
          label="UPTIME"
          value={fmtUptime(health?.uptime_secs ?? 0)}
          color="bone"
          mono
          sub="continuous"
        />
        <MosaicCell
          label="VERSION"
          value={health?.version ?? '—'}
          color="bone"
          mono
          sub="roko-serve"
        />
        <MosaicCell
          label="C-FACTOR"
          value={animComposite.toFixed(3)}
          color="rose"
          mono
          sub={`${cfactor?.composite?.episode_count ?? 0} episodes`}
        />
        <MosaicCell
          label="TOTAL COST"
          value={`$${animCost.toFixed(2)}`}
          color="warning"
          mono
          sub={`$${(efficiency?.cost_per_task ?? 0).toFixed(3)}/task`}
        />
        <MosaicCell
          label="EPISODES"
          value={Math.round(animEpisodes).toLocaleString()}
          color="dream"
          mono
          sub={`${gatesPassed} gates passed`}
        />
      </Mosaic>

      {/* MIDDLE ROW: C-Factor + Model Routing */}
      <div className="dash-flex-row">
        {/* Left: C-Factor breakdown */}
        <div className="dash-flex-1">
          <Pane
            title="C-FACTOR BREAKDOWN"
            badge={<span className="dash-badge--glow">{animComposite.toFixed(3)}</span>}
          >
            <div className="dash-flex-col">
              {METRICS.map((m, i) => {
                const val = subMetrics[m.key] ?? 0;
                const pct = Math.min(val * 100, 100);
                return (
                  <div key={m.key} className={`dash-row-item${i < METRICS.length - 1 ? ' dash-row-sep' : ''}`}>
                    <span className="dash-label-sans-md">{m.label}</span>
                    {/* Bar track */}
                    <div className="dash-bar-track" style={{ flex: 2 }}>
                      <div
                        className="dash-bar-fill dash-bar-fill--rose"
                        style={{
                          width: `${pct}%`,
                          boxShadow: pct > 0 ? '0 0 10px rgba(220,165,189,.45), 0 0 4px rgba(220,165,189,.6)' : 'none',
                        }}
                      />
                    </div>
                    <span
                      className="dash-value--bone"
                      style={{
                        textShadow: pct > 0 ? '0 0 12px rgba(228,216,176,.5)' : 'none',
                      }}
                    >
                      {(val * 100).toFixed(1)}%
                    </span>
                  </div>
                );
              })}
            </div>
          </Pane>
        </div>

        {/* Right: Model Routing */}
        <div className="dash-flex-1">
          <Pane title="MODEL ROUTING" badge={<span className="dash-badge">{currentModel}</span>}>
            <BarChart data={routerBars} height={140} />
          </Pane>
        </div>
      </div>

      {/* BOTTOM ROW: Cost Over Time + Activity */}
      <div className="dash-flex-row">
        {/* Left: Cost Over Time */}
        <div className="dash-flex-1">
          <Pane title="COST OVER TIME" badge={<span className="dash-badge">${totalCost.toFixed(2)} total</span>}>
            <CostChart data={costPoints} height={130} color="var(--bone)" />
          </Pane>
        </div>

        {/* Right: Activity mini stats */}
        <div className="dash-flex-1">
          <Pane title="ACTIVITY" badge={<span className="dash-badge">realtime</span>}>
            <div className="dash-grid-activity">
              <ActivityBlock
                label="Active Plans"
                value={String(health?.active_plans ?? 0)}
                color="var(--bone)"
              />
              <ActivityBlock
                label="Active Agents"
                value={String(health?.active_agents ?? 0)}
                color="var(--rose-bright)"
              />
              <ActivityBlock
                label="Gate Pass Rate"
                value={gatePassRate}
                color="var(--success)"
              />
              <ActivityBlock
                label="Cost/Task"
                value={`$${(efficiency?.cost_per_task ?? 0).toFixed(3)}`}
                color="var(--warning)"
              />
            </div>
          </Pane>
        </div>
      </div>

      {/* C-FACTOR TREND */}
      <Pane
        title="C-FACTOR TREND"
        badge={<span className="dash-badge">24h window</span>}
      >
        <CFactorSparkline
          trend={cfactorTrend?.trend ?? []}
          woolley={cfactorTrend?.woolley}
          height={180}
        />
      </Pane>

      {/* PROVIDER HEALTH */}
      <Pane
        title="PROVIDER HEALTH"
        badge={
          <span className="dash-badge">
            {providerHealth ? `${healthyProviders}/${providers.length} healthy` : 'loading'}
          </span>
        }
      >
        <div className="dash-grid-auto">
          {providers.map((p) => (
            <ProviderCell key={p.name} provider={p} />
          ))}
        </div>
      </Pane>

      {/* ADAPTIVE THRESHOLDS */}
      <Pane
        title="ADAPTIVE GATE THRESHOLDS"
        badge={<span className="dash-badge">7-rung EMA</span>}
      >
        <ThresholdGaugeRow thresholds={thresholds?.thresholds ?? {}} />
      </Pane>
    </div>
  );
}

/* ── Activity block helper ───────────────────────────────── */

function ActivityBlock({ label, value, color }: { label: string; value: string; color: string }) {
  return (
    <div className="dash-card">
      <span className="dash-label-sm">{label}</span>
      <span
        className="dash-value--lg"
        style={{ color, textShadow: `0 0 16px ${color}50` }}
      >
        {value}
      </span>
    </div>
  );
}

/* ── Provider health helpers ─────────────────────────────── */

function ProviderStat({ label, value, color }: { label: string; value: string; color: string }) {
  return (
    <div className="dash-flex-col--gap2">
      <span className="dash-provider-label">{label}</span>
      <span className="dash-provider-value" style={{ color }}>{value}</span>
    </div>
  );
}

function ProviderCell({ provider }: { provider: Provider }) {
  const dot = STATUS_DOT_STYLES[provider.status] ?? STATUS_DOT_STYLES.unhealthy;
  const successColor = provider.success_rate >= 0.97
    ? 'var(--success)'
    : provider.success_rate >= 0.93
      ? 'var(--warning)'
      : 'var(--rose-bright)';
  const latencyColor = provider.avg_latency_ms < 1500
    ? 'var(--success)'
    : provider.avg_latency_ms < 3000
      ? 'var(--warning)'
      : 'var(--rose-bright)';
  const errorsColor = provider.errors_24h < 10
    ? 'var(--text-dim)'
    : provider.errors_24h < 30
      ? 'var(--warning)'
      : 'var(--rose-bright)';

  return (
    <div className="dash-card--lg">
      <div className="dash-card__header">
        <span className="dash-display-name">{provider.name}</span>
        <span
          className="dash-dot--7"
          style={{
            background: dot.bg,
            boxShadow: dot.glow,
            animation: dot.anim,
          }}
        />
      </div>

      <div className="dash-card__tags">
        {provider.models.map((m) => (
          <span key={m} className="dash-tag--sm">{m}</span>
        ))}
      </div>

      <div className="dash-card__stats">
        <ProviderStat label="Success" value={`${(provider.success_rate * 100).toFixed(1)}%`} color={successColor} />
        <ProviderStat label="Avg Latency" value={`${provider.avg_latency_ms}ms`} color={latencyColor} />
        <ProviderStat
          label="Cost/1K"
          value={provider.cost_per_1k_tokens > 0 ? `$${provider.cost_per_1k_tokens.toFixed(4)}` : 'free'}
          color="var(--bone-bright)"
        />
        <ProviderStat label="Errors (24h)" value={String(provider.errors_24h)} color={errorsColor} />
        <ProviderStat label="Requests" value={provider.total_requests.toLocaleString()} color="var(--text-soft)" />
        <ProviderStat label="p95 Latency" value={`${provider.p95_latency_ms}ms`} color="var(--text-dim)" />
      </div>
    </div>
  );
}
