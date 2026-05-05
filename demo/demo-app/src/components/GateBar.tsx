import { CheckmarkIcon, CrossIcon, SpinnerIcon } from './icons/AnimatedIcons';
import './GateBar.css';

interface Gate {
  name: string;
  status: 'pass' | 'fail' | 'pending' | 'skip';
}

interface GateBarProps {
  gates: Gate[];
}

function GateIcon({ status }: { status: Gate['status'] }) {
  switch (status) {
    case 'pass':  return <CheckmarkIcon size={14} color="var(--success)" />;
    case 'fail':  return <CrossIcon size={14} color="var(--rose-bright)" />;
    case 'pending': return <SpinnerIcon size={14} />;
    default: return <span>{'\u2013'}</span>;
  }
}

export default function GateBar({ gates }: GateBarProps) {
  return (
    <div className="gate-bar">
      {gates.map((g) => (
        <span key={g.name} className={`gate gate-${g.status}`}>
          <span className="gate-icon" aria-label={g.status}><GateIcon status={g.status} /></span>
          {g.name}
        </span>
      ))}
    </div>
  );
}
