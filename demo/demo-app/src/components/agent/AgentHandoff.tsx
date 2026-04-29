import { useEffect, useRef, useState } from 'react';
import AgentAvatar from './AgentAvatar';
import './AgentHandoff.css';

interface AgentInfo {
  name: string;
  role?: string;
  status?: 'idle' | 'working' | 'done';
}

interface AgentHandoffProps {
  from: AgentInfo;
  to: AgentInfo;
  status: 'pending' | 'active' | 'done' | 'error';
  direction?: 'forward' | 'reverse' | 'bidirectional';
  label?: string;
  sublabel?: string;
  artifacts?: number;
  progress?: number;
  compact?: boolean;
  className?: string;
  /** Stagger index for stack animation (0-based). */
  index?: number;
}

/** Number of crystal particles rendered in each direction. */
const PARTICLE_COUNT = 5;

/** Build an array of particle elements for a given flow direction. */
function renderParticles(dir: 'forward' | 'reverse') {
  return Array.from({ length: PARTICLE_COUNT }, (_, i) => (
    <span
      key={`${dir}-${i}`}
      className={`agent-handoff__particle agent-handoff__particle--${dir}`}
    />
  ));
}

export default function AgentHandoff({
  from,
  to,
  status,
  direction = 'forward',
  label,
  sublabel,
  artifacts,
  progress,
  compact = false,
  className,
  index = 0,
}: AgentHandoffProps) {
  const showForward = direction === 'forward' || direction === 'bidirectional';
  const showReverse = direction === 'reverse' || direction === 'bidirectional';

  /* ── Typewriter label reveal ── */
  const [revealedLabel, setRevealedLabel] = useState('');
  const labelRef = useRef(label);
  useEffect(() => {
    labelRef.current = label;
    if (!label || status === 'pending') {
      setRevealedLabel(label ?? '');
      return;
    }
    setRevealedLabel('');
    let i = 0;
    const id = setInterval(() => {
      i++;
      if (i > (labelRef.current?.length ?? 0)) {
        clearInterval(id);
        return;
      }
      setRevealedLabel(labelRef.current?.slice(0, i) ?? '');
    }, 32);
    return () => clearInterval(id);
  }, [label, status]);

  const rootClasses = [
    'agent-handoff',
    compact ? 'agent-handoff--compact' : '',
    className,
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <div
      className={rootClasses}
      data-status={status}
      style={{ '--handoff-stagger': `${index * 80}ms` } as React.CSSProperties}
    >
      {/* ── Left agent (source) ── */}
      <div
        className="agent-handoff__card agent-handoff__card--source"
        data-agent-status={from.status ?? 'idle'}
      >
        <AgentAvatar
          name={from.name}
          role={from.role}
          size={compact ? 'xs' : 'sm'}
        />
        <span className="agent-handoff__name">{from.name}</span>
        {from.role && (
          <span className="agent-handoff__role">{from.role}</span>
        )}
      </div>

      {/* ── Flow zone ── */}
      <div className="agent-handoff__flow">
        {/* Connection beam + particles */}
        <div className="agent-handoff__connection">
          <div className="agent-handoff__line" />
          <div className="agent-handoff__beam-glow" />
          {showForward && (
            <span className="agent-handoff__arrow agent-handoff__arrow--forward" />
          )}
          {showReverse && (
            <span className="agent-handoff__arrow agent-handoff__arrow--reverse" />
          )}
          <div className="agent-handoff__particles">
            {showForward && renderParticles('forward')}
            {showReverse && renderParticles('reverse')}
          </div>
          <div className="agent-handoff__settled-glow" />
        </div>

        {/* Label with typewriter */}
        {label && (
          <span className="agent-handoff__label">
            {status === 'done' && (
              <span className="agent-handoff__check">{'\u2713'} </span>
            )}
            <span className="agent-handoff__label-text">{revealedLabel}</span>
            {revealedLabel.length < (label?.length ?? 0) && (
              <span className="agent-handoff__cursor" />
            )}
          </span>
        )}

        {/* Sublabel + artifacts */}
        {(sublabel || artifacts != null) && (
          <div className="agent-handoff__meta">
            {sublabel && (
              <span className="agent-handoff__sublabel">{sublabel}</span>
            )}
            {artifacts != null && (
              <span className="agent-handoff__artifacts">
                {artifacts} artifact{artifacts !== 1 ? 's' : ''}
              </span>
            )}
          </div>
        )}

        {/* Progress bar */}
        {progress != null && (
          <div className="agent-handoff__progress">
            <div
              className="agent-handoff__progress-fill"
              style={{ width: `${Math.round(Math.max(0, Math.min(1, progress)) * 100)}%` }}
            />
          </div>
        )}
      </div>

      {/* ── Right agent (target) ── */}
      <div
        className="agent-handoff__card agent-handoff__card--target"
        data-agent-status={to.status ?? 'idle'}
      >
        <AgentAvatar
          name={to.name}
          role={to.role}
          size={compact ? 'xs' : 'sm'}
        />
        <span className="agent-handoff__name">{to.name}</span>
        {to.role && (
          <span className="agent-handoff__role">{to.role}</span>
        )}
        {/* Connected badge on completion */}
        {status === 'done' && (
          <span className="agent-handoff__connected-badge">Connected</span>
        )}
      </div>
    </div>
  );
}
