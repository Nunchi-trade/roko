import { useEffect, useState } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import type { PipelineStage } from '../lib/pipeline-types';
import { scaleIn } from '../design/motion-tokens';
import './ActivityStrip.css';

export interface ActivityStripProps {
  stage?: PipelineStage;
  connected?: boolean;
  activeAgents?: number;
  lastEventAt?: number;
}

/** Map pipeline stages to visual categories for the pill. */
function stageCategory(stage: PipelineStage): 'idle' | 'running' | 'error' | 'done' {
  switch (stage) {
    case 'idle':
    case 'selecting':
    case 'configuring':
      return 'idle';
    case 'failed':
      return 'error';
    case 'complete':
      return 'done';
    default:
      return 'running';
  }
}

/** Human-readable label for the stage pill. */
function stageLabel(stage: PipelineStage): string {
  switch (stage) {
    case 'idle':           return 'IDLE';
    case 'selecting':      return 'SELECT';
    case 'configuring':    return 'CONFIG';
    case 'starting':       return 'STARTING';
    case 'prd_generating': return 'PRD';
    case 'planning':       return 'PLAN';
    case 'executing':      return 'EXEC';
    case 'gate_checking':  return 'GATES';
    case 'paused':         return 'PAUSED';
    case 'failed':         return 'FAILED';
    case 'complete':       return 'DONE';
  }
}

/** Format a timestamp as relative time (e.g. "2s ago"). */
function relativeTime(ts: number, now: number): string {
  const delta = Math.max(0, Math.floor((now - ts) / 1000));
  if (delta < 60) return `${delta}s ago`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  return `${Math.floor(delta / 3600)}h ago`;
}

export default function ActivityStrip({
  stage = 'idle',
  connected = false,
  activeAgents = 0,
  lastEventAt,
}: ActivityStripProps) {
  const cat = stageCategory(stage);

  // Tick relative time every second when we have a timestamp
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (lastEventAt == null) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [lastEventAt]);

  return (
    <div className="activity-strip" role="status" aria-label="Activity status">
      {/* Pipeline stage pill */}
      <AnimatePresence mode="wait" initial={false}>
        <motion.span
          key={stage}
          className={`activity-strip__stage activity-strip__stage--${cat}`}
          initial={scaleIn.initial}
          animate={scaleIn.animate}
          exit={scaleIn.exit}
        >
          {stageLabel(stage)}
        </motion.span>
      </AnimatePresence>

      <span className="activity-strip__sep" aria-hidden="true" />

      {/* Server connection dot */}
      <span className="activity-strip__server">
        <span
          className={`activity-strip__dot activity-strip__dot--${connected ? 'connected' : 'disconnected'}`}
          aria-label={connected ? 'Server connected' : 'Server disconnected'}
        />
        <span>{connected ? 'ONLINE' : 'OFFLINE'}</span>
      </span>

      <span className="activity-strip__sep" aria-hidden="true" />

      {/* Active agents */}
      <span className="activity-strip__agents">
        <span className="activity-strip__agents-count">{activeAgents}</span>
        {' '}agent{activeAgents !== 1 ? 's' : ''} active
      </span>

      {/* Last event timestamp */}
      {lastEventAt != null && (
        <span className="activity-strip__event">
          {relativeTime(lastEventAt, now)}
        </span>
      )}
    </div>
  );
}
