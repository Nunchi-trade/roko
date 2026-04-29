import { useState, useCallback, useRef, useEffect } from 'react';
import { CheckmarkIcon, CrossIcon } from '../../components/icons/AnimatedIcons';

function useCountUp(target: number, duration = 600): number {
  const [value, setValue] = useState(0);
  const frameRef = useRef<number>(0);
  useEffect(() => {
    const start = performance.now();
    const tick = (now: number) => {
      const progress = Math.min((now - start) / duration, 1);
      const eased = 1 - (1 - progress) * (1 - progress);
      setValue(Math.round(target * eased));
      if (progress < 1) frameRef.current = requestAnimationFrame(tick);
    };
    frameRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frameRef.current);
  }, [target, duration]);
  return value;
}

interface DemoCompletionOverlayProps {
  title: string;
  stats: { model: string; cost: string; tokens: string; time: string };
  gates: { name: string; status: 'pass' | 'fail' | 'pending' }[];
  onDismiss: () => void;
  onRunAgain: () => void;
  onNextScenario: () => void;
  hasNext: boolean;
}

export default function DemoCompletionOverlay({
  title, stats, gates, onDismiss, onRunAgain, onNextScenario, hasNext,
}: DemoCompletionOverlayProps) {
  const [dismissing, setDismissing] = useState(false);
  const passCount = gates.filter((g) => g.status === 'pass').length;
  const failCount = gates.filter((g) => g.status === 'fail').length;
  const animatedPass = useCountUp(passCount, 500);
  const animatedFail = useCountUp(failCount, 500);
  const doDismiss = useCallback(() => { setDismissing(true); setTimeout(() => onDismiss(), 350); }, [onDismiss]);

  return (
    <div className={`demo-completion-overlay${dismissing ? ' dismissing' : ''}`} onClick={doDismiss}>
      <div className="demo-completion-card" onClick={(e) => e.stopPropagation()}>
        <div className="demo-completion-header">
          <span className="demo-completion-title">{title}</span>
          <span className="demo-completion-badge">COMPLETE</span>
        </div>
        <div className="demo-completion-stats">
          {stats.model !== '--' && (
            <div className="demo-completion-stat">
              <span className="demo-completion-stat-label">MODEL</span>
              <span className="demo-completion-stat-value mono">{stats.model}</span>
            </div>
          )}
          {stats.cost !== '--' && (
            <div className="demo-completion-stat">
              <span className="demo-completion-stat-label">COST</span>
              <span className="demo-completion-stat-value mono">{stats.cost}</span>
            </div>
          )}
          {stats.time !== '--' && (
            <div className="demo-completion-stat">
              <span className="demo-completion-stat-label">DURATION</span>
              <span className="demo-completion-stat-value mono">{stats.time}</span>
            </div>
          )}
          {gates.length > 0 && (
            <div className="demo-completion-stat">
              <span className="demo-completion-stat-label">GATES</span>
              <span className="demo-completion-stat-value demo-completion-gates">
                <span className="demo-completion-gate-pass">
                  <CheckmarkIcon size={12} color="var(--success)" />
                  {animatedPass}
                </span>
                {failCount > 0 && (
                  <span className="demo-completion-gate-fail">
                    <CrossIcon size={12} color="var(--rose-bright)" />
                    {animatedFail}
                  </span>
                )}
              </span>
            </div>
          )}
        </div>
        <div className="demo-completion-actions">
          <button className="demo-completion-btn demo-completion-btn-again" onClick={(e) => { e.stopPropagation(); setDismissing(true); setTimeout(() => onRunAgain(), 350); }}>
            Run Again
          </button>
          {hasNext && (
            <button className="demo-completion-btn demo-completion-btn-next" onClick={(e) => { e.stopPropagation(); setDismissing(true); setTimeout(() => onNextScenario(), 350); }}>
              Next Scenario {'\u2192'}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
