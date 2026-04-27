import { useState, useCallback, useRef, useEffect } from 'react';
import { SCENARIOS, type Scenario } from '../lib/demo-scenarios';
import TerminalPane from '../components/Terminal/TerminalPane';
import Timeline from '../components/Timeline';
import StatCard from '../components/StatCard';
import './Demo.css';

export default function Demo() {
  const [activeIdx, setActiveIdx] = useState(0);
  const [currentStep, setCurrentStep] = useState(-1);
  const [running, setRunning] = useState(false);
  const [introCollapsed, setIntroCollapsed] = useState(false);
  const [stats, setStats] = useState({ model: '—', cost: '$0.00', tokens: '0', time: '0s' });
  const [speed, setSpeed] = useState(1);
  const timerRef = useRef(0);

  const scenario = SCENARIOS[activeIdx];
  const sessionIds = scenario.labels.map((_, i) => `demo-${scenario.id}-${i}`);

  const selectScenario = useCallback((idx: number) => {
    setActiveIdx(idx);
    setCurrentStep(-1);
    setRunning(false);
    setIntroCollapsed(false);
    setStats({ model: '—', cost: '$0.00', tokens: '0', time: '0s' });
  }, []);

  const play = useCallback(async () => {
    if (running || scenario.steps.length === 0) return;
    setRunning(true);
    setIntroCollapsed(true);

    for (let i = 0; i < scenario.steps.length; i++) {
      setCurrentStep(i);
      // Simulate step duration
      await new Promise((r) => {
        timerRef.current = window.setTimeout(r, 3000 / speed);
      });
      // Update simulated stats
      setStats((s) => ({
        model: 'claude-sonnet-4',
        cost: `$${(parseFloat(s.cost.slice(1)) + 0.02 + Math.random() * 0.08).toFixed(2)}`,
        tokens: `${parseInt(s.tokens) + Math.floor(200 + Math.random() * 800)}`,
        time: `${(i + 1) * Math.round(3 / speed)}s`,
      }));
    }

    setRunning(false);
  }, [running, scenario, speed]);

  useEffect(() => {
    return () => clearTimeout(timerRef.current);
  }, []);

  const cycleSpeed = () => setSpeed((s) => (s >= 4 ? 0.5 : s * 2));

  const timelineSteps = scenario.steps.map((step, i) => ({
    label: step.label,
    detail: step.sublabel,
    status: (i < currentStep ? 'done' : i === currentStep ? 'active' : 'pending') as 'done' | 'active' | 'pending',
  }));

  return (
    <div className="demo-page">
      {/* Scenario tabs */}
      <div className="demo-tabs-bar">
        <div className="demo-tab-list">
          {SCENARIOS.map((s, i) => (
            <button
              key={s.id}
              className={`demo-tab${activeIdx === i ? ' active' : ''}`}
              onClick={() => selectScenario(i)}
            >
              <span className="demo-tab-num">{i + 1}</span>
              {s.title}
            </button>
          ))}
        </div>
        <div className="demo-controls">
          <button className="demo-speed" onClick={cycleSpeed}>{speed}×</button>
          <button className="btn-primary" onClick={play} disabled={running || scenario.steps.length === 0}>
            {running ? '▶ Running...' : '▶ Play'}
          </button>
        </div>
      </div>

      {/* Intro band */}
      <div className={`demo-intro${introCollapsed ? ' collapsed' : ''}`}>
        <span className="demo-intro-title">{scenario.title}</span>
        <span className="demo-intro-sub">{scenario.subtitle}</span>
      </div>

      {/* Main content */}
      <div className="demo-main">
        <div className="demo-terminals">
          <TerminalZone scenario={scenario} sessionIds={sessionIds} />
        </div>

        {scenario.panel && (
          <div className="demo-panel">
            {/* Canvas placeholder */}
            <div className="demo-canvas">
              <canvas id="demo-viz" />
            </div>

            {/* Stats */}
            <div className="demo-stats">
              <StatCard label="Model" value={stats.model} color="rose" />
              <StatCard label="Cost" value={stats.cost} color="bone" />
              <StatCard label="Tokens" value={stats.tokens} color="sage" />
              <StatCard label="Time" value={stats.time} color="warn" />
            </div>

            {/* Timeline */}
            {timelineSteps.length > 0 && <Timeline steps={timelineSteps} />}
          </div>
        )}
      </div>
    </div>
  );
}

/** Terminal zone renders 1, 2, or 4 panes depending on the scenario. */
function TerminalZone({ scenario, sessionIds }: { scenario: Scenario; sessionIds: string[] }) {
  const cols = scenario.panes === 4 ? 2 : scenario.panes === 2 ? 2 : 1;

  return (
    <div className={`demo-terminal-grid demo-cols-${cols}`}>
      {sessionIds.map((id, i) => (
        <TerminalPane key={id} sessionId={id} label={scenario.labels[i]} />
      ))}
    </div>
  );
}
