import { useState, useEffect } from 'react';
import { useApiWithFallback } from '../hooks/useApiWithFallback';
import type { BenchRun } from '../lib/bench-types';
import { DEMO_BENCH_RUNS } from '../lib/bench-demo-data';
import Pane from '../components/Pane';
import ConfigDiff from '../components/ConfigDiff';
import './Bench.css';

export default function BenchCompare() {
  const { get } = useApiWithFallback();
  const [runs, setRuns] = useState<BenchRun[]>([]);
  const [selectedA, setSelectedA] = useState<string>('');
  const [selectedB, setSelectedB] = useState<string>('');

  useEffect(() => {
    (async () => {
      try {
        const data = await get<BenchRun[]>('/api/bench/runs');
        if (Array.isArray(data) && data.length > 0) {
          setRuns(data);
          return;
        }
      } catch { /* fallback */ }
      setRuns(DEMO_BENCH_RUNS);
    })();
  }, [get]);

  // Auto-select first two runs
  useEffect(() => {
    if (runs.length >= 2 && !selectedA && !selectedB) {
      setSelectedA(runs[0].id);
      setSelectedB(runs[1].id);
    } else if (runs.length === 1 && !selectedA) {
      setSelectedA(runs[0].id);
    }
  }, [runs, selectedA, selectedB]);

  const runA = runs.find((r) => r.id === selectedA);
  const runB = runs.find((r) => r.id === selectedB);

  return (
    <div className="bench-page">
      <div className="bench-hero">
        <div className="bench-hero-header">
          <h1 className="bench-page-title">Compare Runs</h1>
          <p className="bench-page-sub">Side-by-side comparison of benchmark configurations and results</p>
        </div>
      </div>

      <div className="bench-body">
        <Pane title="SELECT RUNS">
          <div className="compare-selectors">
            <div className="compare-select">
              <label className="param-label">Run A</label>
              <select
                className="config-input"
                value={selectedA}
                onChange={(e) => setSelectedA(e.target.value)}
              >
                <option value="">Select run...</option>
                {runs.map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.id.slice(0, 8)} - {r.suite_name} ({r.config.model.split('-').slice(0, 2).join('-')})
                  </option>
                ))}
              </select>
            </div>
            <div className="compare-select">
              <label className="param-label">Run B</label>
              <select
                className="config-input"
                value={selectedB}
                onChange={(e) => setSelectedB(e.target.value)}
              >
                <option value="">Select run...</option>
                {runs.map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.id.slice(0, 8)} - {r.suite_name} ({r.config.model.split('-').slice(0, 2).join('-')})
                  </option>
                ))}
              </select>
            </div>
          </div>
        </Pane>

        {runA && runB ? (
          <Pane title="COMPARISON">
            <ConfigDiff runA={runA} runB={runB} />
          </Pane>
        ) : (
          <div className="bench-empty">
            <p className="bench-empty-text">
              Select two runs above to compare their configurations and results.
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
