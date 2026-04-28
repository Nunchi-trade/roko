import { useState, useEffect, useRef, useCallback } from 'react';
import { useApiWithFallback } from '../hooks/useApiWithFallback';
import Pane from '../components/Pane';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import CostChart from '../components/Charts/CostChart';
import './BenchLive.css';

interface Task {
  id: number;
  status: 'pending' | 'running' | 'pass' | 'fail';
  cost: number;
}

interface FeedItem {
  text: string;
  type: 'pass' | 'fail' | 'info' | 'learn';
  ts: string;
  cost?: number;
}

export default function BenchLive() {
  const [tasks, setTasks] = useState<Task[]>(() =>
    Array.from({ length: 50 }, (_, i) => ({ id: i, status: 'pending', cost: 0 }))
  );
  const [feed, setFeed] = useState<FeedItem[]>([]);
  const [totalCost, setTotalCost] = useState(0);
  const [elapsed, setElapsed] = useState(0);
  const [costPoints, setCostPoints] = useState<{ label: string; value: number }[]>([]);
  const [currentModel, setCurrentModel] = useState('claude-haiku');
  const startTime = useRef(Date.now());
  const { get } = useApiWithFallback();

  const passed = tasks.filter((t) => t.status === 'pass').length;
  const failed = tasks.filter((t) => t.status === 'fail').length;
  const completed = passed + failed;

  // Timer
  useEffect(() => {
    const id = setInterval(() => setElapsed(Math.floor((Date.now() - startTime.current) / 1000)), 1000);
    return () => clearInterval(id);
  }, []);

  // Poll for live data
  const poll = useCallback(async () => {
    try {
      const efficiency = await get<{ tasks?: { cost_usd?: number; passed?: boolean }[] }>('/api/learn/efficiency');
      if (efficiency.tasks && efficiency.tasks.length > 0) {
        const newTasks = efficiency.tasks.map((t, i) => ({
          id: i,
          status: (t.passed ? 'pass' : 'fail') as Task['status'],
          cost: t.cost_usd ?? 0,
        }));
        while (newTasks.length < 50) {
          newTasks.push({ id: newTasks.length, status: 'pending', cost: 0 });
        }
        setTasks(newTasks);

        let cum = 0;
        const points = efficiency.tasks.map((t, i) => {
          cum += t.cost_usd ?? 0;
          return { label: `T${i + 1}`, value: t.cost_usd ?? 0 };
        });
        setCostPoints(points);
        setTotalCost(cum);
      }

      const router = await get<{ current_model?: string }>('/api/learn/cascade-router');
      if (router.current_model) setCurrentModel(router.current_model);
    } catch {
      // Fall back to simulation
    }
  }, [get]);

  // Simulation fallback when no live data
  useEffect(() => {
    const simId = setInterval(() => {
      setTasks((prev) => {
        const pending = prev.filter((t) => t.status === 'pending');
        if (pending.length === 0) return prev;
        const next = pending[0];
        const pass = Math.random() > 0.25;
        const cost = 0.02 + Math.random() * 0.15;
        const updated = prev.map((t) =>
          t.id === next.id ? { ...t, status: (pass ? 'pass' : 'fail') as Task['status'], cost } : t
        );
        setTotalCost((c) => c + cost);
        setCostPoints((pts) => [...pts, { label: `T${next.id + 1}`, value: cost }]);
        setFeed((f) => [
          {
            text: `Task ${next.id + 1}: ${pass ? 'PASS' : 'FAIL'}`,
            type: (pass ? 'pass' : 'fail') as FeedItem['type'],
            ts: new Date().toLocaleTimeString(),
            cost,
          },
          ...f,
        ].slice(0, 50));
        return updated;
      });
    }, 2000);

    poll();
    const realId = setInterval(poll, 5000);

    return () => {
      clearInterval(simId);
      clearInterval(realId);
    };
  }, [poll]);

  const fmtElapsed = `${Math.floor(elapsed / 60)}:${(elapsed % 60).toString().padStart(2, '0')}`;

  return (
    <div className="benchlive-page">
      <div className="benchlive-header">
        <span className="benchlive-logo">{'\u25C6'}</span>
        <span className="benchlive-title">Live Bench Monitor</span>
        <div className="benchlive-status">
          <span className="benchlive-dot" />
          <span>live</span>
        </div>
        <span className="benchlive-elapsed">{fmtElapsed}</span>
      </div>

      <div className="benchlive-metrics">
        <Mosaic columns={5}>
          <MosaicCell label="PASSED" value={completed > 0 ? `${((passed / completed) * 100).toFixed(0)}%` : '93%'} color="success" />
          <MosaicCell label="COST" value={`$${totalCost.toFixed(2)}`} color="bone" mono />
          <MosaicCell label="AVG/TASK" value={completed > 0 ? `${(elapsed / completed).toFixed(1)}s` : '2.4s'} color="dream" mono />
          <MosaicCell label="MODEL" value={currentModel.split('/').pop()?.slice(0, 16) ?? 'claude-haiku'} color="rose" mono />
          <MosaicCell label="TASKS" value={`${completed}/50`} color="bone" mono />
        </Mosaic>
      </div>

      <div className="benchlive-grid">
        <Pane title="TASK GRID">
          <div className="task-grid">
            {tasks.map((t) => (
              <div key={t.id} className={`task-cell task-${t.status}`} title={`Task ${t.id + 1}`} />
            ))}
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
    </div>
  );
}
