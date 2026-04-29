import { useState, useEffect } from 'react';
import { Link } from 'react-router';
import { useApiWithFallback } from '../hooks/useApiWithFallback';
import { useServerHealth } from '../hooks/useServerHealth';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import Pane from '../components/Pane';
import CrushedBar from '../components/CrushedBar';
import PhosphorNumber from '../components/PhosphorNumber';
import ConnectScreen from '../components/ConnectScreen';

/* ================================================================
   Injected keyframes (Landing-only animations)
   ================================================================ */
const LANDING_STYLES = `
  @keyframes landing-breathe {
    0%, 100% { box-shadow: 0 0 6px 1px rgba(138,156,134,.5), 0 0 12px 2px rgba(138,156,134,.2); opacity: 1; }
    50%       { box-shadow: 0 0 10px 3px rgba(138,156,134,.8), 0 0 22px 6px rgba(138,156,134,.35); opacity: .85; }
  }
  @keyframes landing-step-glow {
    0%, 100% { opacity: 1; width: 20px; }
    50%       { opacity: .6; width: 32px; }
  }
  @keyframes landing-flicker {
    0%,  93%, 100% { opacity: 1; }
    94%             { opacity: .6; }
    95%             { opacity: 1; }
    97%             { opacity: .4; }
    98%             { opacity: 1; }
  }
`;

/* ================================================================
   Constants
   ================================================================ */

const LOOP_STEPS = ['query', 'score', 'route', 'compose', 'act', 'verify', 'write', 'react'] as const;

const STEP_DESCRIPTIONS: Record<string, string> = {
  query: 'Ingest signals from any source',
  score: 'Rank by relevance and urgency',
  route: 'Select the optimal model and agent',
  compose: 'Assemble 9-layer system prompts',
  act: 'Execute with tool access',
  verify: '7-rung gate pipeline validates output',
  write: 'Persist results and episodes',
  react: 'Learn, adapt thresholds, replan if needed',
};

const FEATURES = [
  {
    icon: '\u2B21',
    title: 'Coordinate',
    description: 'Route tasks to the right agent and model. 5+ LLM backends, skill-based matching, reputation scoring.',
  },
  {
    icon: '\u25C8',
    title: 'Validate',
    description: '7-rung gate pipeline. Compile, test, lint, diff. Adaptive thresholds that learn from failures.',
  },
  {
    icon: '\u25C9',
    title: 'Learn',
    description: 'Cascade router learns model performance per task type. Cost goes down, quality goes up.',
  },
  {
    icon: '\u27F2',
    title: 'Self-Host',
    description: 'Roko develops itself. From idea to running code, fully automated with checkpoint/resume.',
  },
];

/* ================================================================
   Data interfaces
   ================================================================ */

interface HealthData {
  status?: string;
  uptime_secs?: number;
  version?: string;
  active_agents?: number;
  statehub?: {
    snapshot?: {
      cost_usd_total?: number;
      episodes_total?: number;
      gates_passed?: number;
      gates_failed?: number;
    };
  };
}

interface GatesSummary {
  pass_rate?: number;
}

interface CFactorData {
  composite?: { overall?: number };
}

/* ================================================================
   Component
   ================================================================ */

export default function Landing() {
  const health = useServerHealth();
  const { get } = useApiWithFallback();
  const isConnected = health === 'connected';

  // Metrics
  const [agents, setAgents] = useState(5);
  const [episodes, setEpisodes] = useState(847);
  const [gatePass, setGatePass] = useState(93.1);
  const [cost, setCost] = useState(1.42);
  const [cFactor, setCFactor] = useState(0.847);
  const [version, setVersion] = useState<string | null>(null);

  // Loop animation
  const [activeStep, setActiveStep] = useState(0);

  useEffect(() => {
    const id = setInterval(() => {
      setActiveStep((s) => (s + 1) % LOOP_STEPS.length);
    }, 2200);
    return () => clearInterval(id);
  }, []);

  // Fetch live data
  useEffect(() => {
    (async () => {
      try {
        const h = await get<HealthData>('/api/health');
        if (h.active_agents != null) setAgents(h.active_agents);
        if (h.version) setVersion(h.version);
        const snap = h.statehub?.snapshot;
        if (snap) {
          if (snap.episodes_total != null && snap.episodes_total > 0) setEpisodes(snap.episodes_total);
          if (snap.cost_usd_total != null && snap.cost_usd_total > 0) setCost(snap.cost_usd_total);
          if (snap.gates_passed != null && snap.gates_failed != null) {
            const total = snap.gates_passed + snap.gates_failed;
            if (total > 0) setGatePass(Math.round((snap.gates_passed / total) * 1000) / 10);
          }
        }
      } catch { /* demo fallback */ }

      try {
        const g = await get<GatesSummary>('/api/gates/summary');
        if (g.pass_rate != null && g.pass_rate > 0) setGatePass(Math.round(g.pass_rate * 1000) / 10);
      } catch { /* demo fallback */ }

      try {
        const cf = await get<CFactorData>('/api/metrics/c_factor');
        if (cf.composite?.overall != null && cf.composite.overall > 0) setCFactor(cf.composite.overall);
      } catch { /* demo fallback */ }
    })();
  }, [get]);

  return (
    <div style={{ minHeight: '100vh', overflow: 'auto' }}>
      {!isConnected && <ConnectScreen />}
      <style>{LANDING_STYLES}</style>

      {/* ================================================================
          HERO — 100vh
          ================================================================ */}
      <section style={{
        minHeight: '100vh',
        display: 'flex',
        flexDirection: 'column',
        justifyContent: 'center',
        padding: '0 40px',
        maxWidth: 960,
        margin: '0 auto',
        position: 'relative',
      }}>
        {/* Eyebrow */}
        <div style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          fontFamily: 'var(--mono)',
          fontSize: 10,
          letterSpacing: '.22em',
          textTransform: 'uppercase',
          color: 'var(--text-soft)',
          marginBottom: 32,
        }}>
          <span style={{
            width: 6,
            height: 6,
            borderRadius: '50%',
            background: isConnected ? 'var(--success)' : 'var(--text-dim)',
            boxShadow: isConnected ? '0 0 6px 1px rgba(138,156,134,.5), 0 0 12px 2px rgba(138,156,134,.2)' : 'none',
            display: 'inline-block',
            animation: isConnected ? 'landing-breathe 2.8s ease-in-out infinite' : 'none',
          }} />
          <span>{isConnected ? 'connected' : 'offline'}</span>
        </div>

        {/* Title */}
        <h1 style={{
          fontFamily: 'var(--display)',
          fontStyle: 'italic',
          fontSize: 'clamp(56px, 8vw, 96px)',
          fontWeight: 300,
          color: 'var(--text-strong)',
          lineHeight: 0.92,
          letterSpacing: '-0.03em',
          marginBottom: 28,
          textShadow: '0 0 80px rgba(232,181,206,.08)',
        }}>
          the agent<br />
          <span style={{ color: 'var(--rose-glow)', textShadow: '0 0 40px rgba(232,181,206,.3)' }}>
            coordination
          </span>{' '}plane
        </h1>

        {/* Subtitle */}
        <p style={{
          fontFamily: 'var(--sans)',
          fontWeight: 400,
          fontSize: 16,
          lineHeight: 1.8,
          color: 'var(--text-soft)',
          maxWidth: 520,
          marginBottom: 48,
          letterSpacing: '.01em',
        }}>
          Roko coordinates agent fleets, routes models by cost, gates every output, and learns from results. 18 crates. One universal loop.
        </p>

        {/* CTA buttons */}
        <div style={{ display: 'flex', gap: 16, alignItems: 'center', flexWrap: 'wrap' }}>
          <Link to="/demo" className="btn" style={{ textDecoration: 'none' }}>
            Watch Demo <span className="arr">&rarr;</span>
          </Link>
          <Link to="/dashboard" className="btn bone" style={{ textDecoration: 'none' }}>
            Dashboard
          </Link>
        </div>
      </section>

      {/* Cinematic divider */}
      <div style={{
        height: 1,
        background: 'linear-gradient(to right, transparent 0%, var(--rose-dim) 35%, var(--rose-bright) 50%, var(--rose-dim) 65%, transparent 100%)',
        opacity: 0.35,
        margin: '0 0',
      }} />

      {/* ================================================================
          MOSAIC METRICS STRIP — full width 5-col
          ================================================================ */}
      <section style={{ padding: 0 }}>
        <Mosaic columns={5}>
          <MosaicCell
            label="AGENTS"
            value={<PhosphorNumber value={agents} />}
            color="rose"
          />
          <MosaicCell
            label="EPISODES"
            value={<PhosphorNumber value={episodes} />}
            sub="logged"
            color="bone"
          />
          <MosaicCell
            label="GATE PASS"
            value={<PhosphorNumber value={gatePass} format={(n) => `${n}%`} />}
            color="success"
          />
          <MosaicCell
            label="COST"
            value={<PhosphorNumber value={cost} format={(n) => `$${n.toFixed(2)}`} />}
            sub="cascade-routed"
            color="rose"
          />
          <MosaicCell
            label="C-FACTOR"
            value={<PhosphorNumber value={cFactor} format={(n) => n.toFixed(3)} />}
            sub="composite quality"
            color="dream"
          />
        </Mosaic>
      </section>

      {/* ================================================================
          UNIVERSAL LOOP — 8-step animated strip
          ================================================================ */}
      <section className="reveal" style={{
        padding: '120px 40px 80px',
        maxWidth: 960,
        margin: '0 auto',
      }}>
        <div className="stag">
          <span className="num">01</span>
          <span className="label">The Universal Loop</span>
        </div>

        <div style={{
          display: 'flex',
          gap: 0,
          flexWrap: 'wrap',
          alignItems: 'stretch',
          background: 'rgba(8,8,12,.45)',
          border: '1px solid var(--glass-border)',
          overflow: 'hidden',
        }}>
          {LOOP_STEPS.map((step, i) => {
            const isActive = i === activeStep;
            return (
              <div key={step} style={{
                display: 'flex',
                alignItems: 'center',
                flex: '1 0 auto',
              }}>
                <div style={{
                  padding: '20px 14px 16px',
                  fontFamily: 'var(--mono)',
                  fontSize: 11,
                  letterSpacing: '.14em',
                  color: isActive ? 'var(--rose-glow)' : 'var(--text-dim)',
                  textTransform: 'uppercase',
                  textAlign: 'center',
                  flex: 1,
                  transition: 'color 0.6s ease, background 0.6s ease',
                  background: isActive
                    ? 'radial-gradient(ellipse 80% 70% at 50% 50%, rgba(220,165,189,.10) 0%, rgba(220,165,189,.03) 60%, transparent 100%), linear-gradient(to bottom, rgba(220,165,189,.06), transparent)'
                    : 'transparent',
                  position: 'relative',
                  minHeight: 70,
                  display: 'flex',
                  flexDirection: 'column',
                  alignItems: 'center',
                  justifyContent: 'center',
                  gap: 4,
                }}>
                  {step}
                  <span style={{
                    fontFamily: 'var(--sans)',
                    fontSize: 8,
                    letterSpacing: '.04em',
                    textTransform: 'none',
                    color: isActive ? 'var(--text-soft)' : 'transparent',
                    transition: 'color 0.6s ease',
                    lineHeight: 1.3,
                    maxWidth: 100,
                    fontWeight: 400,
                  }}>
                    {STEP_DESCRIPTIONS[step]}
                  </span>
                  {isActive && (
                    <span style={{
                      position: 'absolute',
                      bottom: 0,
                      left: '50%',
                      transform: 'translateX(-50%)',
                      height: 3,
                      background: 'var(--rose-glow)',
                      boxShadow: '0 0 12px 2px rgba(220,165,189,.7), 0 0 24px 4px rgba(220,165,189,.3)',
                      animation: 'landing-step-glow 1.4s ease-in-out infinite',
                    }} />
                  )}
                </div>
                {i < LOOP_STEPS.length - 1 && (
                  <span style={{
                    color: isActive ? 'var(--rose-glow)' : 'var(--text-dim)',
                    textShadow: isActive ? '0 0 8px rgba(220,165,189,.5)' : 'none',
                    fontSize: 9,
                    padding: '0 2px',
                    userSelect: 'none',
                    transition: 'color 0.6s ease, text-shadow 0.6s ease',
                  }}>&rarr;</span>
                )}
              </div>
            );
          })}
        </div>

        <p style={{
          fontFamily: 'var(--sans)',
          fontWeight: 400,
          fontSize: 13,
          lineHeight: 1.7,
          color: 'var(--text-soft)',
          marginTop: 20,
          letterSpacing: '.02em',
        }}>
          1 noun (Signal) + 6 verb traits. Every agent follows the same loop.
        </p>
      </section>

      {/* ================================================================
          COST COMPARISON
          ================================================================ */}
      <section className="reveal" style={{
        padding: '60px 40px 80px',
        maxWidth: 960,
        margin: '0 auto',
      }}>
        <div className="stag">
          <span className="num">02</span>
          <span className="label">Cost Efficiency</span>
        </div>

        <div style={{ maxWidth: 440 }}>
          <CrushedBar
            naiveLabel="naive single-model"
            naiveValue={44.86}
            actualLabel="cascade-routed"
            actualValue={1.42}
          />
        </div>
      </section>

      {/* ================================================================
          FEATURE GRID — 2x2 with Pane components
          ================================================================ */}
      <section className="reveal" style={{
        padding: '60px 40px 100px',
        maxWidth: 960,
        margin: '0 auto',
      }}>
        <div className="stag">
          <span className="num">03</span>
          <span className="label">What Roko Does</span>
        </div>

        <div style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(2, 1fr)',
          gap: 16,
        }}>
          {FEATURES.map((f) => (
            <Pane key={f.title} title={f.title} badge={<span style={{ fontSize: 16, color: 'var(--rose-dim)' }}>{f.icon}</span>}>
              <div style={{
                fontFamily: 'var(--display)',
                fontSize: 14,
                fontWeight: 300,
                lineHeight: 1.7,
                color: 'var(--text-soft)',
              }}>
                {f.description}
              </div>
            </Pane>
          ))}
        </div>
      </section>

      {/* ================================================================
          FOOTER CTA
          ================================================================ */}
      <section className="reveal" style={{
        padding: '80px 40px 140px',
        maxWidth: 960,
        margin: '0 auto',
        textAlign: 'center',
      }}>
        <div style={{
          fontFamily: 'var(--display)',
          fontStyle: 'italic',
          fontSize: 'clamp(28px, 4vw, 42px)',
          fontWeight: 300,
          color: 'var(--text-strong)',
          lineHeight: 1.15,
          marginBottom: 48,
          letterSpacing: '-0.02em',
        }}>
          the loop is{' '}
          <em className="bloom-rose" style={{ fontStyle: 'italic', animation: 'landing-flicker 7s ease-in-out infinite' }}>running</em>
        </div>

        <div style={{
          display: 'flex',
          justifyContent: 'center',
          gap: 12,
          flexWrap: 'wrap',
          marginBottom: 40,
        }}>
          <Link to="/dashboard" className="btn" style={{ textDecoration: 'none' }}>Dashboard</Link>
          <Link to="/demo" className="btn" style={{ textDecoration: 'none' }}>Demo</Link>
          <Link to="/bench" className="btn" style={{ textDecoration: 'none' }}>Bench</Link>
          <Link to="/explorer" className="btn" style={{ textDecoration: 'none' }}>Explorer</Link>
        </div>

        {version && (
          <p style={{
            fontFamily: 'var(--mono)',
            fontSize: 9,
            color: 'var(--text-dim)',
            letterSpacing: '.22em',
            textTransform: 'uppercase',
          }}>
            roko serve {version}
          </p>
        )}
      </section>
    </div>
  );
}
