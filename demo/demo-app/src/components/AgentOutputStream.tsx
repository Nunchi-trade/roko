import { useEffect, useRef, useMemo } from 'react';
import { stripAnsi } from '../lib/strip-ansi';
import './AgentOutputStream.css';

export interface AgentOutputStreamProps {
  lines: string[];
  agentId: string | null;
}

/* ── Line classification ─────────────────────────────────── */

type LineType = 'success' | 'error' | 'warning' | 'info' | 'muted' | 'filtered';

/** Internal roko probe commands and shell wrapper noise to suppress entirely. */
const FILTERED_PATTERNS = [
  /^.*__rk_ec=\$\?/,
  /^.*__ROKO_/,
  /^.*printf\s+'\\033\]7777/,
  /^\s*\(exit\s+\$__rk_ec\)/,
  /^.*\\033\]7777;D;/,
  /^\s*$/,
];

/** Patterns that indicate success. */
const SUCCESS_PATTERNS = [
  /\bpass(ed)?\b/i,
  /\bcomplete(d)?\b/i,
  /\bsucce(ss|eded)\b/i,
  /\bgate\b.*\bpass/i,
  /\btask\s+completed/i,
  /\bdone\b/i,
];

/** Patterns that indicate errors. */
const ERROR_PATTERNS = [
  /\bfail(ed|ure)?\b/i,
  /\berror\b/i,
  /\bpanic\b/i,
  /\bgate\b.*\bfail/i,
  /\bagent\s+.*\bfailed/i,
  /\brejected\b/i,
  /\babort(ed)?\b/i,
];

/** Patterns that indicate warnings. */
const WARNING_PATTERNS = [
  /\bwarn(ing)?\b/i,
  /\bretry(ing)?\b/i,
  /\brate[- ]?limit/i,
  /\btimeout\b/i,
  /\bslow\b/i,
  /\bbackoff\b/i,
];

/** Patterns for muted/dim metadata. */
const MUTED_PATTERNS = [
  /\btokens?:\s*\d/i,
  /\blatency:\s*\d/i,
  /\bcost:\s*\$/i,
  /\b\d+\s*tok\b/i,
  /\b\d+ms\b/,
  /\(\$\d+\.\d+\)/,
  /\bmodel=/i,
  /\btier=T\d/i,
];

/** Patterns for info (tool calls, spawn, dispatch). */
const INFO_PATTERNS = [
  /\bspawn(ed|ing)?\b/i,
  /\bdispatch(ed|ing)?\b/i,
  /\btool[_ ]?call\b/i,
  /\bagent\b.*\bstart/i,
  /\bexecut(e|ing)\b/i,
  /\bpoll(ing)?\b/i,
];

/** Icon prefix per line type. */
const LINE_ICONS: Record<Exclude<LineType, 'filtered'>, string> = {
  success: '\u2713',  // checkmark
  error:   '\u2717',  // X mark
  warning: '\u21BB',  // retry/loop arrow
  info:    '\u2192',  // right arrow
  muted:   '\u00B7',  // middle dot
};

function classifyLine(raw: string): LineType {
  const stripped = stripAnsi(raw).trim();

  // Filter out internal noise
  for (const pat of FILTERED_PATTERNS) {
    if (pat.test(stripped)) return 'filtered';
  }

  // Classify by content
  for (const pat of ERROR_PATTERNS) {
    if (pat.test(stripped)) return 'error';
  }
  for (const pat of SUCCESS_PATTERNS) {
    if (pat.test(stripped)) return 'success';
  }
  for (const pat of WARNING_PATTERNS) {
    if (pat.test(stripped)) return 'warning';
  }
  for (const pat of MUTED_PATTERNS) {
    if (pat.test(stripped)) return 'muted';
  }
  for (const pat of INFO_PATTERNS) {
    if (pat.test(stripped)) return 'info';
  }

  return 'info';
}

/** Parse a line that may contain a bracketed task label like [T1-add-greeting]. */
function parseTaskLabel(text: string): { label: string | null; rest: string } {
  const match = text.match(/^\[([^\]]+)\]\s*(.*)/);
  if (match) return { label: match[1], rest: match[2] };
  return { label: null, rest: text };
}

/* ── Component ───────────────────────────────────────────── */

/** Terminal-style scrolling viewer for live agent output with structured formatting. */
export default function AgentOutputStream({ lines, agentId }: AgentOutputStreamProps) {
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [lines.length]);

  const classified = useMemo(() => {
    return lines
      .map((raw) => {
        const type = classifyLine(raw);
        const cleaned = stripAnsi(raw).trim();
        return { raw, cleaned, type };
      })
      .filter((l) => l.type !== 'filtered' && l.cleaned.length > 0);
  }, [lines]);

  return (
    <div className="agent-output-stream">
      <div className="agent-output-header">
        <span className="agent-output-title">Agent Output</span>
        {agentId && <span className="agent-output-badge">{agentId}</span>}
        {classified.length > 0 && (
          <span className="agent-output-count">{classified.length}</span>
        )}
      </div>

      {classified.length === 0 ? (
        <div className="agent-output-empty">Waiting for agent output...</div>
      ) : (
        <div className="agent-output-body">
          {classified.map((line, i) => {
            const { label, rest } = parseTaskLabel(line.cleaned);
            const icon = LINE_ICONS[line.type as Exclude<LineType, 'filtered'>] ?? '';

            return (
              <div
                key={i}
                className={`agent-output-line agent-output-line--${line.type}`}
              >
                <span className="agent-output-icon">{icon}</span>
                {label && (
                  <span className="agent-output-label">[{label}]</span>
                )}
                <span className="agent-output-text">{rest || line.cleaned}</span>
              </div>
            );
          })}
          <div ref={bottomRef} />
        </div>
      )}
    </div>
  );
}
