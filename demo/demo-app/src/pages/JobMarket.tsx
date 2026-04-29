import { useState } from 'react';
import { useTerminal } from '../hooks/useTerminal';
import { useJobEvents } from '../hooks/useJobEvents';
import JobFlowViz, {
  type JobFlowJobState,
  type JobFlowAgent,
  type JobFlowVotes,
  type JobFlowEvent,
} from '../components/JobFlowViz';
import ChainEvidenceStrip from '../components/ChainEvidenceStrip';
import EfficiencyBar, { type EfficiencyMetric } from '../components/EfficiencyBar';
import '@xterm/xterm/css/xterm.css';
import '../components/Terminal/TerminalPane.css';
import './JobMarket.css';

/* ── Helpers ── */

function mapJobState(hook: ReturnType<typeof useJobEvents>): JobFlowJobState {
  const j = hook.jobs[0];
  if (!j) return { id: 0, state: 'waiting', bounty: '50', accepted: undefined };
  return {
    id: j.id,
    state: j.state === 'funded' ? 'funded'
      : j.state === 'assigned' ? 'assigned'
      : j.state === 'submitted' ? 'submitted'
      : j.state === 'resolved' ? 'resolved'
      : 'waiting',
    bounty: j.bounty,
    accepted: j.accepted,
  };
}

function mapAgents(hook: ReturnType<typeof useJobEvents>): [JobFlowAgent, JobFlowAgent] {
  const poster = hook.agents.find(a => a.role === 'poster');
  const worker = hook.agents.find(a => a.role === 'worker');
  return [
    {
      name: poster?.name ?? 'ALPHA',
      role: 'poster' as const,
      reputation: poster?.reputation ?? 500000,
      tier: poster?.tier ?? 'Standard',
      active: true,
    },
    {
      name: worker?.name ?? 'BETA',
      role: 'worker' as const,
      reputation: worker?.reputation ?? 500000,
      tier: worker?.tier ?? 'Standard',
      active: true,
    },
  ];
}

function mapVotes(hook: ReturnType<typeof useJobEvents>): JobFlowVotes {
  return {
    voters: hook.votes.voters.map(v => ({ approve: v.approve })),
    verdict: hook.votes.verdict,
  };
}

function buildFlowEvents(hook: ReturnType<typeof useJobEvents>): JobFlowEvent[] {
  return hook.events
    .filter((e): e is import('../hooks/useJobEvents').JobEvent =>
      'id' in e && ['posted', 'assigned', 'submitted', 'vote', 'resolved'].includes(e.type))
    .map(e => {
      switch (e.type) {
        case 'posted': return { type: 'posted' as const, bounty: e.bounty ?? '50' };
        case 'assigned': return { type: 'assigned' as const };
        case 'submitted': return { type: 'submitted' as const };
        case 'vote': return { type: 'vote' as const, approve: e.approve ?? true };
        case 'resolved': return { type: 'resolved' as const, accepted: e.accepted ?? true };
        default: return { type: 'assigned' as const };
      }
    });
}

/* ── Page ── */

export default function JobMarket() {
  const jobEvents = useJobEvents();

  // Derive viz props
  const job = mapJobState(jobEvents);
  const agents = mapAgents(jobEvents);
  const votes = mapVotes(jobEvents);
  const flowEvents = buildFlowEvents(jobEvents);

  // Metrics
  const [metrics] = useState<EfficiencyMetric[]>([
    { label: 'COST', value: 0, format: (n) => `$${n.toFixed(2)}`, color: 'bone' },
    { label: 'TOKENS', value: 0, color: 'dream' },
    { label: 'MODEL', value: 0, format: () => 'claude-sonnet', color: 'rose' },
    { label: 'ELAPSED', value: 0, format: () => '0:00', color: 'dream' },
  ]);

  // Track which terminal is active (for rose border)
  const [activeTerm] = useState<'alpha' | 'beta' | null>(null);

  return (
    <div className="job-market-page">
      {/* Header bar with connection status + run button */}
      <div className="job-market-header">
        <div className="job-market-header-left">
          <span className={`job-market-status-dot ${jobEvents.connected ? 'live' : 'off'}`} />
          <span className="job-market-status-label">
            {jobEvents.connected ? 'CHAIN CONNECTED' : 'CHAIN OFFLINE'}
          </span>
        </div>
        <button
          className={`job-market-run-btn ${jobEvents.demoRunning ? 'running' : ''}`}
          onClick={jobEvents.runDemo}
          disabled={jobEvents.demoRunning || !jobEvents.connected}
          title={!jobEvents.connected ? 'Start mirage-rs to enable live demo' : undefined}
        >
          {jobEvents.demoRunning ? 'RUNNING…' : 'RUN DEMO'}
        </button>
      </div>

      {/* Main 3-column layout */}
      <div className="job-market-main">
        {/* Left terminal: Alpha / Poster */}
        <div className={`job-market-term${activeTerm === 'alpha' ? ' active' : ''}`}>
          <TermPane
            sessionId="job-market-alpha"
            label="ALPHA · poster"
          />
        </div>

        {/* Center: Flow Visualization */}
        <div className="job-market-viz">
          <JobFlowViz
            job={job}
            agents={agents}
            votes={votes}
            events={flowEvents}
            jobTitle="Research Uniswap V4 Gas Optimization"
          />
        </div>

        {/* Right terminal: Beta / Worker */}
        <div className={`job-market-term${activeTerm === 'beta' ? ' active' : ''}`}>
          <TermPane
            sessionId="job-market-beta"
            label="BETA · worker"
          />
        </div>
      </div>

      {/* Chain evidence strip */}
      <ChainEvidenceStrip txs={jobEvents.chainTxs} />

      {/* Bottom metrics */}
      <div className="job-market-bottom">
        <EfficiencyBar metrics={metrics} />
      </div>
    </div>
  );
}

/* ── Terminal sub-component ── */

function TermPane({ sessionId, label }: { sessionId: string; label: string }) {
  const { attach, status } = useTerminal(sessionId);

  return (
    <>
      <div className="job-market-term-head">
        <span className={`demo-term-dot ${status}`} />
        <span>{label}</span>
      </div>
      <div className="job-market-term-body" ref={attach} />
    </>
  );
}
