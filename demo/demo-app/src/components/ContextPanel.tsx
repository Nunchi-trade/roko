import { useState } from 'react';

// ── Types ────────────────────────────────────────────────────

export type ContextPanelStage =
  | 'init'
  | 'idea'
  | 'draft'
  | 'promote'
  | 'plan'
  | 'validate'
  | 'run'
  | 'done';

export interface ContextPanelProps {
  stage: ContextPanelStage;
  idea?: string;
  prd?: { title: string; requirements: string[]; acceptance: string[] };
  plan?: { tasks: { name: string; role: string; status: string }[] };
  gates?: { name: string; status: 'pass' | 'fail' | 'pending' }[];
  summary?: { tasksCompleted: string; cost: string; time: string };
}

// ── Collapsible section ───────────────────────────────────────

function Collapsible({ label, count, children }: { label: string; count: number; children: React.ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="cp-collapsible">
      <button className="cp-collapsible-toggle" onClick={() => setOpen(v => !v)}>
        <span>{label}</span>
        <span className="cp-badge">{count}</span>
        <span className="cp-chevron">{open ? '▲' : '▼'}</span>
      </button>
      {open && <div className="cp-collapsible-body">{children}</div>}
    </div>
  );
}

// ── Gate badge ───────────────────────────────────────────────

function GateBadge({ name, status }: { name: string; status: 'pass' | 'fail' | 'pending' }) {
  const cls = status === 'pass' ? 'cp-gate-pass' : status === 'fail' ? 'cp-gate-fail' : 'cp-gate-pending';
  return (
    <span className={`cp-gate-badge ${cls}`}>
      {status === 'pass' ? '✓' : status === 'fail' ? '✗' : '○'} {name}
    </span>
  );
}

// ── Stage content ─────────────────────────────────────────────

function StageContent({ stage, idea, prd, plan, gates, summary }: ContextPanelProps) {
  switch (stage) {
    case 'init':
      return (
        <div className="cp-stage-content">
          <p className="cp-hint">Workspace created. Ready to capture an idea.</p>
        </div>
      );

    case 'idea':
      return (
        <div className="cp-stage-content">
          {idea ? (
            <blockquote className="cp-idea-quote">{idea}</blockquote>
          ) : (
            <p className="cp-hint">Idea captured.</p>
          )}
        </div>
      );

    case 'draft':
      return (
        <div className="cp-stage-content">
          {prd ? (
            <>
              <div className="cp-prd-title">{prd.title}</div>
              <Collapsible label="Requirements" count={prd.requirements.length}>
                <ul className="cp-list">
                  {prd.requirements.map((r, i) => <li key={i}>{r}</li>)}
                </ul>
              </Collapsible>
              <Collapsible label="Acceptance" count={prd.acceptance.length}>
                <ul className="cp-list">
                  {prd.acceptance.map((a, i) => <li key={i}>{a}</li>)}
                </ul>
              </Collapsible>
            </>
          ) : (
            <p className="cp-hint">PRD generated.</p>
          )}
        </div>
      );

    case 'promote':
      return (
        <div className="cp-stage-content">
          <p className="cp-hint">PRD published. Ready for plan generation.</p>
        </div>
      );

    case 'plan':
      return (
        <div className="cp-stage-content">
          {plan && plan.tasks.length > 0 ? (
            <table className="cp-task-table">
              <thead>
                <tr>
                  <th>Task</th>
                  <th>Role</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody>
                {plan.tasks.map((t, i) => (
                  <tr key={i}>
                    <td>{t.name}</td>
                    <td className="cp-dim">{t.role}</td>
                    <td>
                      <span className={`cp-task-status cp-task-${t.status}`}>{t.status}</span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <p className="cp-hint">Implementation plan generated.</p>
          )}
        </div>
      );

    case 'validate':
      return (
        <div className="cp-stage-content">
          <p className="cp-hint cp-hint-ok">Plan valid. Ready for execution.</p>
        </div>
      );

    case 'run':
      return (
        <div className="cp-stage-content">
          {gates && gates.length > 0 ? (
            <div className="cp-gate-list">
              {gates.map((g) => (
                <GateBadge key={g.name} name={g.name} status={g.status} />
              ))}
            </div>
          ) : (
            <p className="cp-hint">Executing plan\u2026</p>
          )}
        </div>
      );

    case 'done':
      return (
        <div className="cp-stage-content">
          {summary ? (
            <div className="cp-summary-card">
              <div className="cp-summary-row">
                <span className="cp-summary-label">Tasks</span>
                <span className="cp-summary-value">{summary.tasksCompleted}</span>
              </div>
              <div className="cp-summary-row">
                <span className="cp-summary-label">Cost</span>
                <span className="cp-summary-value">{summary.cost}</span>
              </div>
              <div className="cp-summary-row">
                <span className="cp-summary-label">Time</span>
                <span className="cp-summary-value">{summary.time}</span>
              </div>
            </div>
          ) : (
            <p className="cp-hint cp-hint-ok">Pipeline complete.</p>
          )}
        </div>
      );

    default:
      return null;
  }
}

// ── Stage label ───────────────────────────────────────────────

const STAGE_LABELS: Record<ContextPanelStage, string> = {
  init: 'Init',
  idea: 'Idea',
  draft: 'PRD Draft',
  promote: 'Promote',
  plan: 'Plan',
  validate: 'Validate',
  run: 'Execute',
  done: 'Done',
};

// ── ContextPanel ──────────────────────────────────────────────

export function ContextPanel(props: ContextPanelProps) {
  const { stage } = props;
  const label = STAGE_LABELS[stage] ?? stage;

  return (
    <div className="cp-panel">
      <div className="cp-header">
        <span className="cp-stage-label">{label}</span>
      </div>
      <StageContent {...props} />

      <style>{`
        .cp-panel {
          font-family: var(--font-mono, monospace);
          font-size: 12px;
          color: rgba(255,255,255,0.75);
        }
        .cp-header {
          padding: 4px 0 8px 0;
          border-bottom: 1px solid rgba(255,255,255,0.08);
          margin-bottom: 8px;
        }
        .cp-stage-label {
          font-size: 10px;
          font-family: var(--font-sans, sans-serif);
          text-transform: uppercase;
          letter-spacing: 0.08em;
          color: rgba(255,255,255,0.4);
        }
        .cp-stage-content {
          display: flex;
          flex-direction: column;
          gap: 6px;
        }
        .cp-hint {
          color: rgba(255,255,255,0.4);
          font-size: 11px;
          font-family: var(--font-sans, sans-serif);
          margin: 0;
        }
        .cp-hint-ok {
          color: #4ade80;
        }
        .cp-idea-quote {
          font-family: var(--font-sans, sans-serif);
          font-size: 11px;
          color: rgba(255,255,255,0.8);
          border-left: 2px solid rgba(244,114,182,0.6);
          margin: 0;
          padding: 4px 8px;
          background: rgba(244,114,182,0.05);
          line-height: 1.5;
        }
        .cp-prd-title {
          font-family: var(--font-sans, sans-serif);
          font-size: 11px;
          font-weight: 600;
          color: rgba(255,255,255,0.85);
          margin-bottom: 4px;
        }
        .cp-collapsible {
          border: 1px solid rgba(255,255,255,0.08);
          border-radius: 4px;
          overflow: hidden;
        }
        .cp-collapsible-toggle {
          display: flex;
          align-items: center;
          gap: 6px;
          width: 100%;
          padding: 5px 8px;
          background: rgba(255,255,255,0.03);
          border: none;
          cursor: pointer;
          font-family: var(--font-sans, sans-serif);
          font-size: 10px;
          color: rgba(255,255,255,0.55);
          text-align: left;
        }
        .cp-collapsible-toggle:hover {
          background: rgba(255,255,255,0.06);
          color: rgba(255,255,255,0.75);
        }
        .cp-badge {
          background: rgba(255,255,255,0.1);
          border-radius: 8px;
          padding: 1px 5px;
          font-size: 9px;
        }
        .cp-chevron {
          margin-left: auto;
          font-size: 8px;
          opacity: 0.5;
        }
        .cp-collapsible-body {
          padding: 6px 8px;
          border-top: 1px solid rgba(255,255,255,0.06);
        }
        .cp-list {
          margin: 0;
          padding-left: 14px;
          font-family: var(--font-sans, sans-serif);
          font-size: 10px;
          color: rgba(255,255,255,0.6);
          display: flex;
          flex-direction: column;
          gap: 3px;
        }
        .cp-task-table {
          width: 100%;
          border-collapse: collapse;
          font-family: var(--font-sans, sans-serif);
          font-size: 10px;
        }
        .cp-task-table th {
          color: rgba(255,255,255,0.3);
          text-align: left;
          padding: 2px 4px;
          border-bottom: 1px solid rgba(255,255,255,0.07);
          font-weight: 500;
        }
        .cp-task-table td {
          padding: 3px 4px;
          color: rgba(255,255,255,0.65);
          border-bottom: 1px solid rgba(255,255,255,0.04);
          white-space: nowrap;
          overflow: hidden;
          text-overflow: ellipsis;
          max-width: 120px;
        }
        .cp-dim {
          color: rgba(255,255,255,0.35) !important;
        }
        .cp-task-status {
          font-size: 9px;
          padding: 1px 4px;
          border-radius: 3px;
        }
        .cp-task-pending { color: rgba(255,255,255,0.3); }
        .cp-task-active { color: #60a5fa; }
        .cp-task-done { color: #4ade80; }
        .cp-task-failed { color: #f87171; }
        .cp-gate-list {
          display: flex;
          flex-wrap: wrap;
          gap: 5px;
        }
        .cp-gate-badge {
          font-size: 10px;
          padding: 2px 6px;
          border-radius: 3px;
          border: 1px solid;
          font-family: var(--font-sans, sans-serif);
        }
        .cp-gate-pass { color: #4ade80; border-color: rgba(74,222,128,0.3); background: rgba(74,222,128,0.07); }
        .cp-gate-fail { color: #f87171; border-color: rgba(248,113,113,0.3); background: rgba(248,113,113,0.07); }
        .cp-gate-pending { color: rgba(255,255,255,0.3); border-color: rgba(255,255,255,0.1); background: transparent; }
        .cp-summary-card {
          display: flex;
          flex-direction: column;
          gap: 5px;
          background: rgba(255,255,255,0.03);
          border: 1px solid rgba(255,255,255,0.08);
          border-radius: 5px;
          padding: 8px;
        }
        .cp-summary-row {
          display: flex;
          justify-content: space-between;
          align-items: center;
          font-family: var(--font-sans, sans-serif);
          font-size: 11px;
        }
        .cp-summary-label {
          color: rgba(255,255,255,0.4);
        }
        .cp-summary-value {
          color: rgba(255,255,255,0.85);
          font-weight: 600;
        }
      `}</style>
    </div>
  );
}

export default ContextPanel;
