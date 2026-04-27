import './GateBar.css';

interface Gate {
  name: string;
  status: 'pass' | 'fail' | 'pending' | 'skip';
}

interface GateBarProps {
  gates: Gate[];
}

const ICONS: Record<Gate['status'], string> = {
  pass: '✓',
  fail: '✗',
  pending: '○',
  skip: '–',
};

export default function GateBar({ gates }: GateBarProps) {
  return (
    <div className="gate-bar">
      {gates.map((g) => (
        <span key={g.name} className={`gate gate-${g.status}`}>
          <span className="gate-icon">{ICONS[g.status]}</span>
          {g.name}
        </span>
      ))}
    </div>
  );
}
