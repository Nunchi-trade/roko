import type { BenchSuite } from '../lib/bench-types';

interface SuiteSelectorProps {
  suites: BenchSuite[];
  value: string;
  onChange: (suiteId: string) => void;
}

export default function SuiteSelector({ suites, value, onChange }: SuiteSelectorProps) {
  return (
    <div className="config-cards">
      {suites.map((s) => (
        <button
          key={s.id}
          className={`config-card${value === s.id ? ' selected' : ''}`}
          onClick={() => onChange(s.id)}
        >
          <span className="card-label">{s.name}</span>
          <span className="card-desc">
            {s.tasks.length} tasks &middot; ~${s.estimated_cost_usd.toFixed(2)}
          </span>
          <span className="card-desc" style={{ opacity: 0.7 }}>
            Difficulty {s.difficulty_range[0]}-{s.difficulty_range[1]}
          </span>
        </button>
      ))}
    </div>
  );
}
