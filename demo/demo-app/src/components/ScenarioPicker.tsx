import { useState, useEffect, useRef, useCallback } from 'react';
import { SCENARIO_CATEGORIES, scenariosByCategory, type Scenario } from '../lib/scenarios';
import './ScenarioPicker.css';

export default function ScenarioPicker({
  scenarios,
  activeIdx,
  onSelect,
}: {
  scenarios: Scenario[];
  activeIdx: number;
  onSelect: (idx: number) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const active = scenarios[activeIdx];
  const grouped = scenariosByCategory(scenarios);

  const close = useCallback(() => setOpen(false), []);

  // Close on Escape
  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [open, close]);

  // Close on click-outside
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close();
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open, close]);

  return (
    <div className="scenario-picker" ref={ref}>
      <button className="scenario-picker-trigger" onClick={() => setOpen(!open)}>
        <span className="scenario-picker-cat">{active.category}</span>
        <span className="scenario-picker-title">{active.title}</span>
        <span className={`scenario-picker-chevron${open ? ' open' : ''}`}>&#9662;</span>
      </button>

      {open && (
        <div className="scenario-picker-popover">
          {SCENARIO_CATEGORIES.map((cat) => {
            const items = grouped.get(cat.id) ?? [];
            if (items.length === 0) return null;
            return (
              <div key={cat.id} className="scenario-picker-group">
                <div className="scenario-picker-group-label">{cat.label}</div>
                {items.map((s) => {
                  const idx = scenarios.indexOf(s);
                  return (
                    <button
                      key={s.id}
                      className={`scenario-picker-item${idx === activeIdx ? ' active' : ''}`}
                      onClick={() => { onSelect(idx); close(); }}
                    >
                      <span className="scenario-picker-item-title">{s.title}</span>
                      <span className="scenario-picker-item-sub">{s.subtitle}</span>
                    </button>
                  );
                })}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
