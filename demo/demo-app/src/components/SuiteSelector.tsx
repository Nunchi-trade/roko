import { useState } from 'react';
import type { BenchSuite } from '../lib/bench-types';

interface SuiteSelectorProps {
  suites: BenchSuite[];
  value: string;
  onChange: (suiteId: string) => void;
}

function DifficultyBar({ range }: { range: [number, number] }) {
  return (
    <div className="suite-difficulty-bar">
      {[1, 2, 3, 4, 5].map((d) => (
        <div
          key={d}
          className={`suite-diff-dot${d >= range[0] && d <= range[1] ? ' active' : ''}`}
          style={{
            background: d >= range[0] && d <= range[1]
              ? d <= 2 ? 'var(--success)' : d <= 3 ? 'var(--bone)' : 'var(--rose-bright)'
              : undefined,
          }}
        />
      ))}
    </div>
  );
}

export default function SuiteSelector({ suites, value, onChange }: SuiteSelectorProps) {
  const [expandedId, setExpandedId] = useState<string | null>(null);

  return (
    <div className="suite-selector">
      <div className="suite-grid">
        {suites.map((s) => {
          const isSelected = value === s.id;
          const isExpanded = expandedId === s.id;

          // Difficulty distribution
          const diffCounts = [0, 0, 0, 0, 0];
          for (const t of s.tasks) {
            if (t.difficulty >= 1 && t.difficulty <= 5) diffCounts[t.difficulty - 1]++;
          }
          const maxCount = Math.max(...diffCounts, 1);

          return (
            <div key={s.id} className={`suite-card${isSelected ? ' selected' : ''}`}>
              <button
                className="suite-card-header"
                onClick={() => onChange(s.id)}
              >
                <div className="suite-card-top">
                  <span className="suite-card-name">{s.name}</span>
                  <span className="suite-card-count">{s.tasks.length} tasks</span>
                </div>
                <p className="suite-card-desc">{s.description}</p>
                <div className="suite-card-meta">
                  <span className="suite-card-cost">~${s.estimated_cost_usd.toFixed(2)}</span>
                  <DifficultyBar range={s.difficulty_range} />
                </div>
                <div className="suite-card-dist">
                  {diffCounts.map((count, i) => (
                    <div key={i} className="suite-dist-col">
                      <div className="suite-dist-bar-wrap">
                        <div
                          className="suite-dist-bar"
                          style={{
                            height: `${(count / maxCount) * 100}%`,
                            background: i <= 1 ? 'var(--success)' : i <= 2 ? 'var(--bone)' : 'var(--rose-dim)',
                          }}
                        />
                      </div>
                      <span className="suite-dist-label">D{i + 1}</span>
                    </div>
                  ))}
                </div>
              </button>
              <button
                className="suite-card-expand"
                onClick={(e) => {
                  e.stopPropagation();
                  setExpandedId(isExpanded ? null : s.id);
                }}
              >
                {isExpanded ? 'Hide tasks' : 'Show tasks'}
              </button>
              {isExpanded && (
                <div className="suite-task-list">
                  {s.tasks.map((t) => (
                    <div key={t.id} className="suite-task-item">
                      <span className={`suite-task-diff diff-${t.difficulty}`}>D{t.difficulty}</span>
                      <span className="suite-task-name">{t.name}</span>
                      <div className="suite-task-tags">
                        {t.tags.map((tag) => (
                          <span key={tag} className="suite-task-tag">{tag}</span>
                        ))}
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
