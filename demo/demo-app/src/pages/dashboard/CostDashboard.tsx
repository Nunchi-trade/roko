import { useState, useEffect, useRef } from 'react';
import { useApiWithFallback } from '../../hooks/useApiWithFallback';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import CostChart from '../../components/Charts/CostChart';
import BarChart from '../../components/Charts/BarChart';

/* ── useCountUp: animates from 0 → target on first mount ─── */

function useCountUp(target: number, duration = 900): number {
  const [val, setVal] = useState(0);
  const prevTarget = useRef<number | null>(null);
  useEffect(() => {
    if (target === 0) return;
    if (prevTarget.current === target) return;
    prevTarget.current = target;
    const start = performance.now();
    const from = val;
    const tick = (now: number) => {
      const t = Math.min((now - start) / duration, 1);
      const eased = 1 - Math.pow(1 - t, 3);
      setVal(from + (target - from) * eased);
      if (t < 1) requestAnimationFrame(tick);
      else setVal(target);
    };
    requestAnimationFrame(tick);
  // eslint-disable-next-line react-hooks/exhaustive-deps
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

interface RouterResponse {
  model_slugs?: string[];
  role_table?: Record<string, string>;
  confidence_stats?: Record<string, { successes: number; trials: number; total_cost_usd: number }>;
  total_observations?: number;
}

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
  const { get } = useApiWithFallback();
  const [health, setHealth] = useState<HealthResponse | null>(null);
  const [efficiency, setEfficiency] = useState<EfficiencyResponse | null>(null);
  const [cfactor, setCfactor] = useState<CFactorResponse | null>(null);
  const [router, setRouter] = useState<RouterResponse | null>(null);

  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      const [h, e, c, r] = await Promise.all([
        get<HealthResponse>('/api/health'),
        get<EfficiencyResponse>('/api/learn/efficiency'),
        get<CFactorResponse>('/api/metrics/c_factor'),
        get<RouterResponse>('/api/learn/cascade-router'),
      ]);
      if (cancelled) return;
      setHealth(h);
      setEfficiency(e);
      setCfactor(c);
      setRouter(r);
    };
    poll();
    const id = setInterval(poll, 10_000);
    return () => { cancelled = true; clearInterval(id); };
  }, [get]);

  /* Derived */
  const snap = health?.statehub?.snapshot;
  const totalCost = efficiency?.total_cost ?? snap?.cost_usd_total ?? 1.42;
  const episodes = snap?.episodes_total ?? 847;
  const isOnline = health?.status === 'ok';
  const composite = cfactor?.composite?.overall ?? 0.847;
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
    if (!rt || Object.keys(rt).length === 0) return 'claude-haiku';
    const modelCounts: Record<string, number> = {};
    for (const model of Object.values(rt)) {
      modelCounts[model] = (modelCounts[model] ?? 0) + 1;
    }
    return Object.entries(modelCounts).sort(([, a], [, b]) => b - a)[0]?.[0] ?? 'claude-haiku';
  })();

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16, maxWidth: 1200 }}>
      {/* ═══ Keyframes ═══ */}
      <style>{`
        @keyframes pulse-dot {
          0%, 100% { opacity: 1; }
          50% { opacity: 0.4; }
        }
      `}</style>

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
              {isOnline ? 'Online' : 'Demo'}
            </span>
          }
          color="success"
          sub={health?.providers ? `${health.providers.healthy}/${health.providers.total} providers` : '4/5 providers'}
        />
        <MosaicCell
          label="UPTIME"
          value={fmtUptime(health?.uptime_secs ?? 14523)}
          color="bone"
          mono
          sub="continuous"
        />
        <MosaicCell
          label="VERSION"
          value={health?.version ?? '0.9.2'}
          color="bone"
          mono
          sub="roko-serve"
        />
        <MosaicCell
          label="C-FACTOR"
          value={animComposite.toFixed(3)}
          color="rose"
          mono
          sub={`${cfactor?.composite?.episode_count ?? 847} episodes`}
        />
        <MosaicCell
          label="TOTAL COST"
          value={`$${animCost.toFixed(2)}`}
          color="warning"
          mono
          sub={`$${(efficiency?.cost_per_task ?? 0.017).toFixed(3)}/task`}
        />
        <MosaicCell
          label="EPISODES"
          value={Math.round(animEpisodes).toLocaleString()}
          color="dream"
          mono
          sub={`${snap?.gates_passed ?? 791} gates passed`}
        />
      </Mosaic>

      {/* ═══ MIDDLE ROW: C-Factor + Model Routing ═══ */}
      <div style={{ display: 'flex', gap: 16 }}>
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
                    padding: '10px 0',
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
          <Pane title="MODEL ROUTING" badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 10 }}>{currentModel}</span>}>
            <BarChart data={routerBars} height={200} />
          </Pane>
        </div>
      </div>

      {/* ═══ BOTTOM ROW: Cost Over Time + Activity ═══ */}
      <div style={{ display: 'flex', gap: 16 }}>
        {/* Left: Cost Over Time */}
        <div style={{ flex: 1, minWidth: 0 }}>
          <Pane title="COST OVER TIME" badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 10 }}>${totalCost.toFixed(2)} total</span>}>
            <CostChart data={costPoints} height={180} color="var(--bone)" />
          </Pane>
        </div>

        {/* Right: Activity mini stats */}
        <div style={{ flex: 1, minWidth: 0 }}>
          <Pane title="ACTIVITY" badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 10 }}>realtime</span>}>
            <div style={{
              display: 'grid',
              gridTemplateColumns: '1fr 1fr',
              gap: 12,
            }}>
              <ActivityBlock
                label="Active Plans"
                value={String(health?.active_plans ?? 2)}
                color="var(--bone)"
              />
              <ActivityBlock
                label="Active Agents"
                value={String(health?.active_agents ?? 5)}
                color="var(--rose-bright)"
              />
              <ActivityBlock
                label="Gate Pass Rate"
                value={`${(((snap?.gates_passed ?? 791) / ((snap?.gates_passed ?? 791) + (snap?.gates_failed ?? 56))) * 100).toFixed(1)}%`}
                color="var(--success)"
              />
              <ActivityBlock
                label="Cost/Task"
                value={`$${(efficiency?.cost_per_task ?? 0.017).toFixed(3)}`}
                color="var(--warning)"
              />
            </div>
          </Pane>
        </div>
      </div>
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
      padding: '16px 18px',
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
