import './TerminalPreview.css';

interface TerminalPreviewProps {
  panes: 1 | 2 | 4;
  labels: string[];
  accent?: string;
}

/** Fake terminal lines per label — scenario-relevant hints. */
const FAKE_LINES: Record<string, string[]> = {
  'roko commands': ['$ roko prd idea "wire prompt builder"', '$ roko prd plan system-prompt'],
  'full pipeline': ['$ roko plan run plans/ --resume', '$ roko status'],
  'naive (no replan)': ['$ roko run "task..." --no-replan', '  dispatching gpt-4...'],
  'cascade (full pipeline)': ['$ roko plan run plans/', '  gate: compile... pass'],
  'task execution': ['$ roko run "implement auth"', '  agent: claude-opus'],
  'gate status': ['$ roko learn all', '  gates: 3/3 pass'],
  'zhipu (glm-4)': ['$ roko run --model glm-4', '  tokens: 1.2k'],
  'openai (gpt-5.4-mini)': ['$ roko run --model gpt-5.4-mini', '  tokens: 980'],
  'anthropic (haiku)': ['$ roko run --model haiku', '  tokens: 1.1k'],
  'moonshot (v1)': ['$ roko run --model moonshot-v1', '  tokens: 1.4k'],
};

const DEFAULT_LINES = ['$ roko run "..."', '  dispatching...'];

function getFakeLines(label: string): string[] {
  const key = label.toLowerCase();
  return FAKE_LINES[key] ?? DEFAULT_LINES;
}

export default function TerminalPreview({ panes, labels, accent }: TerminalPreviewProps) {
  return (
    <div className="terminal-preview" data-panes={panes}>
      {Array.from({ length: panes }, (_, i) => {
        const label = labels[i] ?? `pane ${i + 1}`;
        const lines = getFakeLines(label);
        const isLast = i === panes - 1;
        const accentStyle = accent ? { '--tp-accent': accent } as React.CSSProperties : undefined;

        return (
          <div key={i} className="terminal-preview-pane" style={accentStyle}>
            <div className="terminal-preview-header">
              <span className="tp-dot" />
              <span className="tp-label">{label}</span>
            </div>
            <div className="terminal-preview-body">
              {lines.map((line, li) => (
                <div key={li} className="tp-line">
                  {line.startsWith('$') ? (
                    <>
                      <span className="tp-prompt">$ </span>
                      <span className="tp-cmd">{line.slice(2)}</span>
                    </>
                  ) : (
                    <span className="tp-cmd">{line}</span>
                  )}
                </div>
              ))}
              {isLast && <span className="terminal-preview-cursor" />}
            </div>
          </div>
        );
      })}
    </div>
  );
}
