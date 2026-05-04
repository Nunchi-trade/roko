import { useEffect, useRef, useState } from 'react';

// ── Types ────────────────────────────────────────────────────

export interface CommandItem {
  id: string;
  command: string;
  description: string;
  status: 'pending' | 'running' | 'success' | 'failure';
  elapsed?: number;
  error?: string;
}

interface CommandListProps {
  commands: CommandItem[];
  onRun: (id: string) => void;
  onRetry: (id: string) => void;
}

// ── Spinner icon ─────────────────────────────────────────────

function SpinnerIcon() {
  return (
    <svg
      className="cl-spinner"
      viewBox="0 0 16 16"
      width="12"
      height="12"
      aria-hidden="true"
    >
      <circle
        cx="8" cy="8" r="5.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeDasharray="20 14"
      />
    </svg>
  );
}

// ── Elapsed counter (live while running) ─────────────────────

function ElapsedCounter({ startedAt }: { startedAt: number }) {
  const [elapsed, setElapsed] = useState(0);
  const rafRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    rafRef.current = setInterval(() => {
      setElapsed(Date.now() - startedAt);
    }, 200);
    return () => {
      if (rafRef.current) clearInterval(rafRef.current);
    };
  }, [startedAt]);

  return (
    <span className="cl-elapsed-live">{(elapsed / 1000).toFixed(1)}s</span>
  );
}

// ── Single command row ────────────────────────────────────────

function CommandRow({
  item,
  stepNum,
  isNext,
  onRun,
  onRetry,
}: {
  item: CommandItem;
  stepNum: number;
  isNext: boolean;
  onRun: (id: string) => void;
  onRetry: (id: string) => void;
}) {
  const startedAtRef = useRef<number>(Date.now());

  useEffect(() => {
    if (item.status === 'running') {
      startedAtRef.current = Date.now();
    }
  }, [item.status]);

  const statusIcon = () => {
    switch (item.status) {
      case 'success':
        return <span className="cl-icon cl-icon-success" aria-label="success">✓</span>;
      case 'failure':
        return <span className="cl-icon cl-icon-failure" aria-label="failure">✗</span>;
      case 'running':
        return <SpinnerIcon />;
      default:
        return <span className="cl-icon cl-icon-pending" aria-label="pending">○</span>;
    }
  };

  return (
    <div className={`cl-row cl-row-${item.status}${isNext ? ' cl-row-next' : ''}`}>
      <div className="cl-step-num">{stepNum}</div>
      <div className="cl-icon-wrap">{statusIcon()}</div>
      <div className="cl-body">
        <div className="cl-description">{item.description}</div>
        <code className="cl-command">{item.command}</code>
        {item.status === 'running' && (
          <ElapsedCounter startedAt={startedAtRef.current} />
        )}
        {item.status === 'success' && item.elapsed !== undefined && (
          <span className="cl-elapsed-done">{(item.elapsed / 1000).toFixed(1)}s</span>
        )}
        {item.status === 'failure' && (
          <span className="cl-error">{item.error ?? 'Command failed'}</span>
        )}
      </div>
      <div className="cl-action">
        {isNext && item.status === 'pending' && (
          <button
            className="cl-btn cl-btn-run"
            onClick={() => onRun(item.id)}
          >
            Run
          </button>
        )}
        {item.status === 'failure' && (
          <button
            className="cl-btn cl-btn-retry"
            onClick={() => onRetry(item.id)}
          >
            Retry
          </button>
        )}
      </div>
    </div>
  );
}

// ── CommandList ───────────────────────────────────────────────

export function CommandList({ commands, onRun, onRetry }: CommandListProps) {
  // The "next" command is the first pending after all leading successes.
  // If any command has failed, the failed command takes priority for retry.
  const nextPendingId = (() => {
    for (const c of commands) {
      if (c.status === 'failure') return null; // failed cmd gets Retry button directly
      if (c.status === 'pending') return c.id;
      // 'running' or 'success' — keep scanning
    }
    return null;
  })();

  return (
    <div className="cl-list">
      {commands.map((item, i) => (
        <CommandRow
          key={item.id}
          item={item}
          stepNum={i + 1}
          isNext={item.id === nextPendingId}
          onRun={onRun}
          onRetry={onRetry}
        />
      ))}
      {commands.length === 0 && (
        <div className="cl-empty">No commands defined.</div>
      )}

      <style>{`
        .cl-list {
          display: flex;
          flex-direction: column;
          gap: 0;
          font-family: var(--font-mono, monospace);
          font-size: 12px;
        }
        .cl-row {
          display: grid;
          grid-template-columns: 20px 18px 1fr auto;
          align-items: flex-start;
          gap: 6px;
          padding: 8px 6px;
          border-bottom: 1px solid rgba(255,255,255,0.05);
          transition: background 0.15s;
        }
        .cl-row-running {
          background: rgba(255,255,255,0.03);
        }
        .cl-row-next {
          background: rgba(255,255,255,0.04);
        }
        .cl-row-success {
          opacity: 0.65;
        }
        .cl-step-num {
          color: rgba(255,255,255,0.3);
          font-size: 10px;
          padding-top: 1px;
          text-align: right;
        }
        .cl-icon-wrap {
          display: flex;
          align-items: flex-start;
          padding-top: 1px;
        }
        .cl-icon {
          font-size: 11px;
          line-height: 1;
        }
        .cl-icon-pending { color: rgba(255,255,255,0.3); }
        .cl-icon-success { color: #4ade80; }
        .cl-icon-failure { color: #f87171; }
        .cl-spinner {
          animation: cl-spin 0.8s linear infinite;
          color: #60a5fa;
        }
        @keyframes cl-spin {
          to { transform: rotate(360deg); }
        }
        .cl-body {
          display: flex;
          flex-direction: column;
          gap: 2px;
          min-width: 0;
          overflow: hidden;
        }
        .cl-description {
          color: rgba(255,255,255,0.85);
          white-space: nowrap;
          overflow: hidden;
          text-overflow: ellipsis;
          font-family: var(--font-sans, sans-serif);
          font-size: 11px;
        }
        .cl-command {
          color: rgba(255,255,255,0.45);
          font-size: 10px;
          white-space: nowrap;
          overflow: hidden;
          text-overflow: ellipsis;
          display: block;
        }
        .cl-elapsed-live {
          color: #60a5fa;
          font-size: 10px;
        }
        .cl-elapsed-done {
          color: rgba(255,255,255,0.3);
          font-size: 10px;
        }
        .cl-error {
          color: #f87171;
          font-size: 10px;
          white-space: nowrap;
          overflow: hidden;
          text-overflow: ellipsis;
        }
        .cl-action {
          display: flex;
          align-items: flex-start;
          padding-top: 1px;
        }
        .cl-btn {
          font-size: 10px;
          padding: 2px 7px;
          border-radius: 3px;
          border: 1px solid;
          cursor: pointer;
          font-family: var(--font-sans, sans-serif);
          white-space: nowrap;
          transition: background 0.1s;
        }
        .cl-btn-run {
          background: rgba(96,165,250,0.12);
          border-color: rgba(96,165,250,0.4);
          color: #93c5fd;
        }
        .cl-btn-run:hover {
          background: rgba(96,165,250,0.22);
        }
        .cl-btn-retry {
          background: rgba(251,191,36,0.1);
          border-color: rgba(251,191,36,0.35);
          color: #fbbf24;
        }
        .cl-btn-retry:hover {
          background: rgba(251,191,36,0.2);
        }
        .cl-empty {
          color: rgba(255,255,255,0.3);
          font-size: 11px;
          padding: 12px 6px;
          font-family: var(--font-sans, sans-serif);
        }
      `}</style>
    </div>
  );
}

export default CommandList;
