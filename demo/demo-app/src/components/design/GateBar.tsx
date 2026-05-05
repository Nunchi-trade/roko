import type { ReactNode } from 'react';
import { CheckmarkIcon, CrossIcon, SpinnerIcon, PulseIcon } from '../icons/AnimatedIcons';
import './GateBar.css';

type GateStatus = 'pass' | 'fail' | 'running' | 'pending' | 'skipped';

export interface GateResult {
  name: string;
  status: GateStatus;
}

interface GateBarProps {
  gates: GateResult[];
}

function gateIcon(status: GateStatus): ReactNode {
  switch (status) {
    case 'pass': return <CheckmarkIcon size={13} color="var(--success)" />;
    case 'fail': return <CrossIcon size={13} color="var(--rose-bright)" />;
    case 'running': return <SpinnerIcon size={13} />;
    case 'pending': return <PulseIcon size={13} color="var(--text-muted)" />;
    default: return <span>{'\u2014'}</span>;
  }
}

export function GateBar({ gates }: GateBarProps) {
  return (
    <div className="gate-bar">
      {gates.map((gate) => (
        <span
          key={gate.name}
          className={`gate-bar__item gate-bar__item--${gate.status}`}
        >
          <span className="gate-bar__icon">{gateIcon(gate.status)}</span>
          <span className="gate-bar__name">{gate.name}</span>
        </span>
      ))}
    </div>
  );
}
