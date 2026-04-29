import { useState, useEffect, useRef, useCallback } from 'react';
import { useLiveApi } from '../../hooks/useLiveApi';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import CostChart from '../../components/Charts/CostChart';
import CFactorSparkline from '../../components/Charts/CFactorSparkline';
import BarChart from '../../components/Charts/BarChart';
import { ThresholdGaugeRow } from '../../components/ThresholdGauge';
import type { AdaptiveThresholdsResponse } from '../../components/ThresholdGauge';

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

/* ── Helpers ─────────────────────────────────────────────── */

function fmtUptime(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (h === 0) return `${m}m`;
  return `${h}h ${m}m`;
}

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
    <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      {/* ═══ TOP MOSAIC: 6 stats ═══ */}
      <Mosaic columns={6}>
        <MosaicCell
          label="STATUS"
          value={
            <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
              <span style={{
                width: 6, height: 6, borderRadius: '50%',
                background: isOnline ? 'var(--success)' : 'var(--rose-bright)',
                boxShadow: isOnline ? '0 0 6px rgba(122,138,120,.6)' : '0 0 6px rgba(204,144,168,.5)',
                animation: isOnline ? 'pulse-dot 2s ease-in-out infinite' : 'none',
                display: 'inline-block',
              }} />
              <span style={{ fontFamily: 'var(--mono)', letterSpacing: '.04em' }}>{isOnline ? 'Online' : 'Offline'}</span>
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

      {/* ═══ MIDDLE ROW: C-Factor + Model Routing ═══ */}
      <div style={{ display: 'flex', gap: 10 }}>
        {/* Left: C-Factor breakdown */}
        <div style={{ flex: 1, minWidth: 0 }}>
          <Pane
            title="C-FACTOR BREAKDOWN"
            badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 13, fontWeight: 600, textShadow: '0 0 16px rgba(220,165,189,.5)' }}>{animComposite.toFixed(3)}</span>}
          >
            <div style={{ display: 'flex', flexDirection: 'column', gap: 0 }}>
              {METRICS.map((m, i) => {
                const val = subMetrics[m.key] ?? 0;
                const pct = Math.min(val * 100, 100);
                return (
                  <div key={m.key} style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 12,
                    padding: '6px 0',
                    borderBottom: i < METRICS.length - 1 ? '1px solid rgba(255,255,255,.04)' : 'none',
                  }}>
                    <span style={{
                      fontFamily: 'var(--sans)',
                      fontSize: '0.75rem',
                      color: 'var(--text-primary)',
                      flex: 1,
                      minWidth: 0,
                      letterSpacing: '.01em',
                    }}>
                      {m.label}
                    </span>
                    {/* Bar track */}
                    <div style={{
                      flex: 2,
                      height: 4,
                      background: 'rgba(255,255,255,.04)',
                      borderRadius: 2,
                      overflow: 'hidden',
                    }}>
                      <div style={{
                        height: '100%',
                        width: `${pct}%`,
                        background: `linear-gradient(to right, var(--rose-dim), var(--rose-bright))`,
                        borderRadius: 2,
                        transition: 'width .6s cubic-bezier(.22,1,.36,1)',
                        boxShadow: pct > 0 ? '0 0 10px rgba(220,165,189,.45), 0 0 4px rgba(220,165,189,.6)' : 'none',
                      }} />
                    </div>
                    <span style={{
                      fontFamily: 'var(--mono)',
                      fontSize: '0.78rem',
                      fontWeight: 500,
                      color: 'var(--bone-bright)',
                      minWidth: 52,
                      textAlign: 'right',
                      textShadow: pct > 0 ? '0 0 12px rgba(228,216,176,.5)' : 'none',
                    }}>
                      {(val * 100).toFixed(1)}%
                    </span>
                  </div>
                );
              })}
            </div>
          </Pane>
        </div>

        {/* Right: Model Routing */}
        <div style={{ flex: 1, minWidth: 0 }}>
          <Pane title="MODEL ROUTING" badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 13 }}>{currentModel}</span>}>
            <BarChart data={routerBars} height={140} />
          </Pane>
        </div>
      </div>

      {/* ═══ BOTTOM ROW: Cost Over Time + Activity ═══ */}
      <div style={{ display: 'flex', gap: 10 }}>
        {/* Left: Cost Over Time */}
        <div style={{ flex: 1, minWidth: 0 }}>
          <Pane title="COST OVER TIME" badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 13 }}>${totalCost.toFixed(2)} total</span>}>
            <CostChart data={costPoints} height={130} color="var(--bone)" />
          </Pane>
        </div>

        {/* Right: Activity mini stats */}
        <div style={{ flex: 1, minWidth: 0 }}>
          <Pane title="ACTIVITY" badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 13 }}>realtime</span>}>
            <div style={{
              display: 'grid',
              gridTemplateColumns: '1fr 1fr',
              gap: 8,
            }}>
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

      {/* ═══ C-FACTOR TREND ═══ */}
      <Pane
        title="C-FACTOR TREND"
        badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 13 }}>24h window</span>}
      >
        <CFactorSparkline
          trend={cfactorTrend?.trend ?? []}
          woolley={cfactorTrend?.woolley}
          height={180}
        />
      </Pane>

      {/* ═══ PROVIDER HEALTH ═══ */}
      <Pane
        title="PROVIDER HEALTH"
        badge={
          <span style={{ fontFamily: 'var(--mono)', fontSize: 13 }}>
            {providerHealth ? `${healthyProviders}/${providers.length} healthy` : 'loading'}
          </span>
        }
      >
        <div style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fill, minmax(240px, 1fr))',
          gap: 8,
        }}>
          {providers.map((p) => (
            <ProviderCell key={p.name} provider={p} />
          ))}
        </div>
      </Pane>

      {/* ═══ ADAPTIVE THRESHOLDS ═══ */}
      <Pane
        title="ADAPTIVE GATE THRESHOLDS"
        badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 13 }}>7-rung EMA</span>}
      >
        <ThresholdGaugeRow thresholds={thresholds?.thresholds ?? {}} />
      </Pane>
    </div>
  );
}

/* ── Activity block helper ───────────────────────────────── */

function ActivityBlock({ label, value, color }: { label: string; value: string; color: string }) {
  return (
    <div style={{
      background: 'rgba(255,255,255,.04)',
      border: '1px solid rgba(255,255,255,.07)',
      borderRadius: 8,
      padding: '10px 12px',
      display: 'flex',
      flexDirection: 'column',
      gap: 6,
    }}>
      <span style={{
        fontFamily: 'var(--mono)',
        fontSize: '0.55rem',
        letterSpacing: '.14em',
        textTransform: 'uppercase',
        color: 'var(--text-soft)',
      }}>
        {label}
      </span>
      <span style={{
        fontFamily: 'var(--mono)',
        fontSize: '1.2rem',
        fontWeight: 600,
        lineHeight: 1,
        color,
        textShadow: `0 0 16px ${color}50`,
      }}>
        {value}
      </span>
    </div>
  );
}

/* ── Provider health helpers ─────────────────────────────── */

function ProviderStat({ label, value, color }: { label: string; value: string; color: string }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
      <span style={{
        fontFamily: 'var(--mono)',
        fontSize: '0.5rem',
        letterSpacing: '.1em',
        textTransform: 'uppercase',
        color: 'var(--text-dim)',
      }}>
        {label}
      </span>
      <span style={{
        fontFamily: 'var(--mono)',
        fontSize: '0.72rem',
        fontWeight: 500,
        color,
      }}>
        {value}
      </span>
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
    <div style={{
      background: 'rgba(255,255,255,.04)',
      border: '1px solid rgba(255,255,255,.07)',
      borderRadius: 10,
      padding: '10px 12px',
      display: 'flex',
      flexDirection: 'column',
      gap: 6,
    }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
        <span style={{
          fontFamily: 'var(--display)',
          fontSize: 13,
          fontWeight: 500,
          color: 'var(--text-strong)',
          letterSpacing: '.01em',
        }}>
          {provider.name}
        </span>
        <span style={{
          width: 7,
          height: 7,
          borderRadius: '50%',
          background: dot.bg,
          boxShadow: dot.glow,
          animation: dot.anim,
          display: 'inline-block',
          flexShrink: 0,
        }} />
      </div>

      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
        {provider.models.map((m) => (
          <span key={m} style={{
            fontFamily: 'var(--mono)',
            fontSize: 15,
            letterSpacing: '.04em',
            padding: '2px 6px',
            borderRadius: 3,
            background: 'var(--glass-bg)',
            border: '1px solid var(--glass-border)',
            color: 'var(--text-soft)',
          }}>
            {m}
          </span>
        ))}
      </div>

      <div style={{
        display: 'grid',
        gridTemplateColumns: '1fr 1fr',
        gap: '6px 12px',
        borderTop: '1px solid rgba(255,255,255,.04)',
        paddingTop: 8,
      }}>
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
