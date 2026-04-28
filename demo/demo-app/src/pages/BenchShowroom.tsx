import { useState, useEffect, useRef, useCallback } from 'react';
import { DEMO_BENCH_RUNS } from '../lib/bench-demo-data';
import type { BenchRun, BenchTaskResult } from '../lib/bench-types';
import Pane from '../components/Pane';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import CostChart from '../components/Charts/CostChart';
import './Bench.css';

interface Scenario {
  id: string;
  name: string;
  description: string;
  run: BenchRun;
}

const SCENARIOS: Scenario[] = DEMO_BENCH_RUNS.map((run) => ({
  id: run.id,
  name: `${run.suite_name} - ${run.config.model.split('-').slice(0, 2).join(' ')}`,
  description: `${run.config.strategy.replace(/_/g, ' ')} strategy, ${run.results.length} tasks`,
  run,
}));

interface FeedItem {
  text: string;
  type: 'pass' | 'fail' | 'info' | 'start';
  ts: string;
  cost?: number;
}

export default function BenchShowroom() {
  const [selectedScenario, setSelectedScenario] = useState(SCENARIOS[0]?.id ?? '');
  const [playing, setPlaying] = useState(false);
  const [playIndex, setPlayIndex] = useState(0);
  const [visibleResults, setVisibleResults] = useState<BenchTaskResult[]>([]);
  const [feed, setFeed] = useState<FeedItem[]>([]);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const scenario = SCENARIOS.find((s) => s.id === selectedScenario);
  const run = scenario?.run;

  const stop = useCallback(() => {
    if (timerRef.current) {
      clearInterval(timerRef.current);
      timerRef.current = null;
    }
    setPlaying(false);
  }, []);

  const reset = useCallback(() => {
    stop();
    setPlayIndex(0);
    setVisibleResults([]);
    setFeed([]);
  }, [stop]);

  const play = useCallback(() => {
    if (!run) return;
    reset();
    setPlaying(true);
    let idx = 0;

    setFeed([{
      text: `Starting ${run.suite_name} with ${run.config.model}...`,
      type: 'info' as const,
      ts: new Date().toLocaleTimeString(),
    }]);

    timerRef.current = setInterval(() => {
      if (idx >= run.results.length) {
        if (timerRef.current) clearInterval(timerRef.current);
        timerRef.current = null;
        setPlaying(false);
        const ts = new Date().toLocaleTimeString();
        setFeed((f): FeedItem[] => [{
          text: `Completed: ${run.summary?.passed}/${run.summary?.total_tasks} passed`,
          type: 'info' as const,
          ts,
        }, ...f].slice(0, 100));
        return;
      }

      const result = run.results[idx];
      const ts = new Date().toLocaleTimeString();

      setVisibleResults((prev) => [...prev, result]);
      setPlayIndex(idx + 1);
      setFeed((f): FeedItem[] => [{
        text: `${result.task_name}: ${result.status === 'pass' ? 'PASS' : 'FAIL'}`,
        type: (result.status === 'pass' ? 'pass' : 'fail') as FeedItem['type'],
        ts,
        cost: result.cost_usd,
      }, ...f].slice(0, 100));

      idx++;
    }, 800);
  }, [run, reset]);

  // Cleanup on unmount
  useEffect(() => () => { if (timerRef.current) clearInterval(timerRef.current); }, []);

  // Reset when scenario changes
  useEffect(() => { reset(); }, [selectedScenario, reset]);

  // Show all results when not playing and results exist
  const displayResults = playing || visibleResults.length > 0 ? visibleResults : (run?.results ?? []);
  const displayedPassed = displayResults.filter((r) => r.status === 'pass').length;
  const displayedFailed = displayResults.filter((r) => r.status === 'fail').length;
  const displayedCost = displayResults.reduce((s, r) => s + r.cost_usd, 0);
  const total = run?.results.length ?? 0;

  // Compute cost points for chart
  const costPoints = displayResults.map((r, i) => ({
    label: `T${i + 1}`,
    value: r.cost_usd,
  }));

  return (
    <div className="bench-page">
      <div className="bench-hero">
        <div className="bench-hero-header">
          <h1 className="bench-page-title">Bench Showroom</h1>
          <p className="bench-page-sub">Pre-configured demo scenarios with animated playback</p>
        </div>
        <div className="bench-hero-stats">
          <Mosaic columns={4}>
            <MosaicCell label="PASSED" value={String(displayedPassed)} color="success" />
            <MosaicCell label="FAILED" value={String(displayedFailed)} color="warning" />
            <MosaicCell label="COST" value={`$${displayedCost.toFixed(3)}`} color="bone" mono />
            <MosaicCell label="PROGRESS" value={`${displayResults.length}/${total}`} color="rose" />
          </Mosaic>
        </div>
      </div>

      <div className="bench-body">
        <Pane title="SCENARIO">
          <div className="config-cards">
            {SCENARIOS.map((s) => (
              <button
                key={s.id}
                className={`config-card${selectedScenario === s.id ? ' selected' : ''}`}
                onClick={() => setSelectedScenario(s.id)}
              >
                <span className="card-label">{s.name}</span>
                <span className="card-desc">{s.description}</span>
              </button>
            ))}
          </div>
        </Pane>

        <div className="showroom-controls">
          <button className="btn" onClick={play} disabled={playing}>
            {playing ? 'Playing...' : 'Play'}
          </button>
          <button className="btn bone" onClick={stop} disabled={!playing}>
            Stop
          </button>
          <button className="btn bone" onClick={reset}>
            Reset
          </button>
        </div>

        {run && (
          <>
            {(playing || visibleResults.length > 0) && (
              <div className="bench-live-progress-bar" style={{ marginBottom: 16 }}>
                <div
                  className="bench-live-progress-fill"
                  style={{
                    width: `${total > 0 ? (playIndex / total) * 100 : 0}%`,
                    transition: 'width 400ms ease',
                  }}
                />
              </div>
            )}

            <div className="benchlive-grid">
              <Pane title="TASK GRID">
                <div className="task-grid">
                  {Array.from({ length: total }, (_, i) => {
                    const result = displayResults[i];
                    const status = result
                      ? result.status
                      : i === displayResults.length && playing
                        ? 'running'
                        : 'pending';
                    return (
                      <div
                        key={i}
                        className={`task-cell task-${status}`}
                        title={result?.task_name ?? `Task ${i + 1}`}
                      />
                    );
                  })}
                </div>
              </Pane>

              <Pane title="COST CHART">
                <CostChart data={costPoints} height={260} color="var(--bone)" />
              </Pane>

              <Pane title="ACTIVITY FEED">
                <div className="feed-list">
                  {feed.map((item, i) => (
                    <div key={i} className={`feed-item feed-${item.type}`}>
                      <span className="feed-ts">{item.ts}</span>
                      <span className="feed-text">{item.text}</span>
                      {item.cost != null && <span className="feed-cost">${item.cost.toFixed(3)}</span>}
                    </div>
                  ))}
                </div>
              </Pane>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
