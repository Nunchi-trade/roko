import './GateBar.css';

type GateStatus = 'pass' | 'fail' | 'running' | 'pending' | 'skipped';

export interface GateResult {
  name: string;
  status: GateStatus;
}

interface GateBarProps {
  gates: GateResult[];
}

const STATUS_ICONS: Record<GateStatus, string> = {
  pass: '\u2713',       // ✓
  fail: '\u2715',       // ✕
  running: '\u25C9',    // ◉
  pending: '\u25CB',    // ○
  skipped: '\u2014',    // —
};

export function GateBar({ gates }: GateBarProps) {
  return (
    <div className="gate-bar">
      {gates.map((gate) => (
        <span
          key={gate.name}
          className={`gate-bar__item gate-bar__item--${gate.status}`}
        >
          <span className="gate-bar__icon">{STATUS_ICONS[gate.status]}</span>
          <span className="gate-bar__name">{gate.name}</span>
        </span>
      ))}
    </div>
  );
}
