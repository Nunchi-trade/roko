import { useEffect, useState } from 'react';
import { useApiWithFallback } from '../hooks/useApiWithFallback';
import './TelemetrySidebar.css';

const fmt = (n: number) => n.toLocaleString();

interface HealthSnapshot {
  episodes_total?: number;
  gates_passed?: number;
  gates_failed?: number;
  agents_active?: number;
  cost_usd_total?: number;
}

interface HealthResponse {
  status?: string;
  active_agents?: number;
  statehub?: {
    snapshot?: HealthSnapshot;
  };
}

interface CFactorResponse {
  value?: number;
  composite?: { overall?: number };
}

export default function TelemetrySidebar() {
  const { get } = useApiWithFallback();
  const [episodes, setEpisodes] = useState<number | null>(null);
  const [cFactor, setCFactor] = useState<number | null>(null);
  const [agents, setAgents] = useState<number | null>(null);
  const [gatesPassed, setGatesPassed] = useState<number | null>(null);
  const [cost, setCost] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;

    const pollHealth = async () => {
      try {
        const d = await get<HealthResponse>('/api/health');
        if (cancelled) return;
        const snap = d.statehub?.snapshot;
        setEpisodes(snap?.episodes_total ?? 0);
        setAgents(snap?.agents_active ?? d.active_agents ?? 0);
        setGatesPassed(snap?.gates_passed ?? 0);
        setCost(snap?.cost_usd_total ?? 0);
      } catch {
        // server offline -- leave values as-is
      }
    };

    const pollCFactor = async () => {
      try {
        const d = await get<CFactorResponse>('/api/metrics/c_factor');
        if (cancelled) return;
        // Handle both { value } and { composite: { overall } } shapes
        const val = d.value ?? d.composite?.overall;
        if (val != null) setCFactor(val);
      } catch {
        // c-factor endpoint unavailable
      }
    };

    pollHealth();
    pollCFactor();
    const healthId = setInterval(pollHealth, 5_000);
    const cFactorId = setInterval(pollCFactor, 10_000);

    return () => {
      cancelled = true;
      clearInterval(healthId);
      clearInterval(cFactorId);
    };
  }, [get]);

  return (
    <aside className="telemetry-sidebar">
      <div className="tel-row">
        episodes
        <b>{episodes != null ? fmt(episodes) : '—'}</b>
      </div>
      <div className="tel-row">
        c-factor
        <b>{cFactor != null ? cFactor.toFixed(2) : '—'}</b>
      </div>
      <div className="tel-row">
        active agents
        <b>{agents != null ? fmt(agents) : '—'}</b>
      </div>
      <div className="tel-row">
        gates passed
        <b>{gatesPassed != null ? fmt(gatesPassed) : '—'}</b>
      </div>
      <div className="tel-row">
        total cost
        <b>{cost != null ? `$${cost.toFixed(2)}` : '—'}</b>
      </div>
    </aside>
  );
}
