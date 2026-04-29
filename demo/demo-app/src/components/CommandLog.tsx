import { useRef, useEffect, useState, useCallback, useMemo } from 'react';
import { lookupCmdDesc } from '../lib/cmd-descriptions';
import { TraceAnnotation } from './inference';
import './CommandLog.css';

interface LogEntry {
  ts: string;
  text: string;
  type?: 'info' | 'success' | 'error' | 'dim';
  /** T7.60: Optional trace metadata for inference-related log lines. */
  trace?: {
    agentName?: string;
    tier?: 'T0' | 'T1' | 'T2';
    model?: string;
    confidence?: number;
    cost?: number;
  };
}

interface CommandLogProps {
  entries: LogEntry[];
  maxHeight?: string;
}

/**
 * Parse inference metadata from command text.
 * Matches patterns like: [T1] claude-sonnet-4-20250514 1234tok ($0.02)
 * or: model=X tier=T1
 */
function extractTrace(text: string): LogEntry['trace'] | undefined {
  // Bracket pattern: [T0] model-name
  const bracket = text.match(/\[(T[012])\]\s+(\S+)/);
  if (bracket) {
    const costMatch = text.match(/\$([0-9.]+)/);
    return {
      tier: bracket[1] as 'T0' | 'T1' | 'T2',
      model: bracket[2],
      cost: costMatch ? parseFloat(costMatch[1]) : undefined,
    };
  }

  // KV pattern: model=X tier=Y
  const modelMatch = text.match(/model=(\S+)/);
  const tierMatch = text.match(/tier=(T[012])/);
  if (modelMatch && tierMatch) {
    const costMatch = text.match(/cost=([0-9.]+)/);
    return {
      tier: tierMatch[1] as 'T0' | 'T1' | 'T2',
      model: modelMatch[1],
      cost: costMatch ? parseFloat(costMatch[1]) : undefined,
    };
  }

  return undefined;
}

/* ── Relative timestamp formatting ───────────────────── */

function relativeTime(ts: string): string {
  // Parse HH:MM:SS timestamp
  const parts = ts.match(/^(\d{2}):(\d{2}):(\d{2})$/);
  if (!parts) return ts;

  const now = new Date();
  const then = new Date();
  then.setHours(parseInt(parts[1], 10), parseInt(parts[2], 10), parseInt(parts[3], 10), 0);

  const diffSec = Math.floor((now.getTime() - then.getTime()) / 1000);
  if (diffSec < 0 || diffSec > 86400) return ts;
  if (diffSec < 3) return 'just now';
  if (diffSec < 60) return `${diffSec}s ago`;
  const mins = Math.floor(diffSec / 60);
  if (mins < 60) return `${mins}m ago`;
  return ts;
}

/* ── Command syntax highlighting ─────────────────────── */

function HighlightedCommand({ text }: { text: string }) {
  // Only highlight lines starting with "$ roko ..."
  const match = text.match(/^(\$\s+)(roko)(\s+)(\S+)?(.*)?$/);
  if (!match) {
    return <span className="log-text log-text-typewriter log-typewriter-done">{text}</span>;
  }

  const [, prefix, roko, sp1, subcommand, rest] = match;

  // Split rest into flags and arguments
  const parts: React.ReactElement[] = [];
  if (rest) {
    const tokens = rest.match(/\S+/g) || [];
    let idx = 0;
    for (const token of tokens) {
      const key = `t-${idx++}`;
      if (token.startsWith('--') || token.startsWith('-')) {
        parts.push(<span key={key} className="log-syn-flag"> {token}</span>);
      } else {
        parts.push(<span key={key} className="log-syn-arg"> {token}</span>);
      }
    }
  }

  return (
    <span className="log-text log-text-typewriter log-typewriter-done">
      <span className="log-syn-prefix">{prefix}</span>
      <span className="log-syn-roko">{roko}</span>
      {sp1 && subcommand && (
        <>
          {sp1}
          <span className="log-syn-sub">{subcommand}</span>
        </>
      )}
      {parts}
    </span>
  );
}

/* ── SVG status icons ────────────────────────────────── */

function DotIcon() {
  return (
    <span className="log-status-icon">
      <svg viewBox="0 0 14 14" fill="none" className="log-icon-dot">
        <circle cx="7" cy="7" r="2.5" fill="var(--text-dim)" />
      </svg>
    </span>
  );
}

function CheckIcon() {
  return (
    <span className="log-status-icon">
      <svg viewBox="0 0 14 14" fill="none" className="log-icon-check">
        <path d="M3 7.5 L5.5 10 L11 4" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" fill="none" />
      </svg>
    </span>
  );
}

function XIcon() {
  return (
    <span className="log-status-icon">
      <svg viewBox="0 0 14 14" fill="none" className="log-icon-x">
        <path d="M4 4 L10 10 M10 4 L4 10" strokeWidth="1.5" strokeLinecap="round" fill="none" />
      </svg>
    </span>
  );
}

function StatusIcon({ type }: { type: LogEntry['type'] }) {
  switch (type) {
    case 'success': return <CheckIcon />;
    case 'error': return <XIcon />;
    case 'dim': return null;
    case 'info':
    default: return <DotIcon />;
  }
}

/* ── Running spinner (for entries being actively processed) ── */

function RunningIcon() {
  return (
    <span className="log-status-icon">
      <svg viewBox="0 0 14 14" fill="none" className="log-icon-spinner">
        <circle cx="7" cy="7" r="5" stroke="var(--text-dim)" strokeWidth="1.5" strokeDasharray="20 12" />
      </svg>
    </span>
  );
}

/* ── Typewriter text component ───────────────────────── */

function TypewriterText({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  const isLong = text.length > 30;

  useEffect(() => {
    if (!isLong) {
      setDone(true);
      return;
    }
    const timer = setTimeout(() => setDone(true), 220);
    return () => clearTimeout(timer);
  }, [isLong]);

  if (!isLong || done) {
    return <HighlightedCommand text={text} />;
  }

  return (
    <span className={`log-text-typewriter`}>{text}</span>
  );
}

/* ── Copy button ─────────────────────────────────────── */

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(() => {
    // Strip leading "$ " from command text for clipboard
    const clean = text.replace(/^\$\s+/, '');
    navigator.clipboard.writeText(clean).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    });
  }, [text]);

  return (
    <button
      className={`log-copy-btn${copied ? ' log-copy-ok' : ''}`}
      onClick={handleCopy}
      title="Copy command"
      aria-label="Copy command"
    >
      {copied ? (
        <svg viewBox="0 0 14 14" fill="none" width="12" height="12">
          <path d="M3 7.5 L5.5 10 L11 4" stroke="var(--status-success)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      ) : (
        <svg viewBox="0 0 14 14" fill="none" width="12" height="12">
          <rect x="4" y="4" width="7" height="8" rx="1" stroke="currentColor" strokeWidth="1.2" />
          <path d="M3 10V3a1 1 0 011-1h5" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
        </svg>
      )}
    </button>
  );
}

/* ── Group logic ─────────────────────────────────────── */

/** Extract the roko subcommand category from text for grouping. */
function cmdCategory(text: string): string | null {
  const m = text.match(/^\$?\s*(?:\.\/target\/\S+\/)?roko\s+(\S+)/);
  return m ? m[1] : null;
}

interface EntryGroup {
  kind: 'single';
  entry: LogEntry;
  index: number;
}

interface MultiGroup {
  kind: 'group';
  type: LogEntry['type'];
  category: string;
  entries: { entry: LogEntry; index: number }[];
}

type RenderItem = EntryGroup | MultiGroup;

function groupEntries(entries: LogEntry[]): RenderItem[] {
  const items: RenderItem[] = [];
  let i = 0;

  while (i < entries.length) {
    const current = entries[i];
    const currentType = current.type ?? 'info';
    const currentCat = cmdCategory(current.text);

    // Look ahead for consecutive same-type, same-category entries (minimum 3 to group)
    let runEnd = i + 1;
    while (
      runEnd < entries.length &&
      (entries[runEnd].type ?? 'info') === currentType &&
      cmdCategory(entries[runEnd].text) === currentCat
    ) {
      runEnd++;
    }

    const runLen = runEnd - i;

    if (runLen >= 3 && currentCat) {
      const group: MultiGroup = {
        kind: 'group',
        type: currentType,
        category: currentCat,
        entries: [],
      };
      for (let j = i; j < runEnd; j++) {
        group.entries.push({ entry: entries[j], index: j });
      }
      items.push(group);
      i = runEnd;
    } else {
      items.push({ kind: 'single', entry: current, index: i });
      i++;
    }
  }

  return items;
}

/* ── Collapsible group ───────────────────────────────── */

function LogGroup({ group, newestIndex, exitingSet }: {
  group: MultiGroup;
  newestIndex: number;
  exitingSet: Set<number>;
}) {
  const [collapsed, setCollapsed] = useState(false);

  const toggle = useCallback(() => setCollapsed((c) => !c), []);

  const label = group.category
    ? `${group.entries.length} ${group.category} commands`
    : `${group.type ?? 'info'} (${group.entries.length})`;

  return (
    <div className="log-group">
      <div className="log-group-header" onClick={toggle}>
        <span className={`log-group-chevron${collapsed ? ' collapsed' : ''}`}>&#x25BE;</span>
        <span>{label}</span>
      </div>
      <div
        className={`log-group-body${collapsed ? ' collapsed' : ''}`}
        style={collapsed ? undefined : { maxHeight: group.entries.length * 40 }}
      >
        {group.entries.map(({ entry, index }) => (
          <LogEntryRow
            key={index}
            entry={entry}
            index={index}
            isNewest={index === newestIndex}
            isExiting={exitingSet.has(index)}
          />
        ))}
      </div>
    </div>
  );
}

/* ── Description subtitle ────────────────────────────── */

function EntryDescription({ text }: { text: string }) {
  // Strip "$ " prefix to look up the command
  const cmd = text.replace(/^\$\s+/, '');
  const desc = lookupCmdDesc(cmd);
  if (!desc) return null;
  return <div className="log-entry-desc">{desc}</div>;
}

/* ── Single entry row ────────────────────────────────── */

function LogEntryRow({ entry, index, isNewest, isExiting }: {
  entry: LogEntry;
  index: number;
  isNewest: boolean;
  isExiting: boolean;
}) {
  const trace = entry.trace ?? extractTrace(entry.text);
  const entryType = entry.type ?? 'info';
  const isCommand = entry.text.startsWith('$ ');

  const className = [
    'log-entry',
    `log-${entryType}`,
    'log-entering',
    isNewest ? 'log-newest' : '',
    isExiting ? 'log-exiting' : '',
  ].filter(Boolean).join(' ');

  return (
    <div className={className} style={{ animationDelay: `${(index % 5) * 30}ms` }}>
      {isNewest && entryType === 'info' ? <RunningIcon /> : <StatusIcon type={entryType} />}
      <span className="log-ts" title={entry.ts}>{relativeTime(entry.ts)}</span>
      <div className="log-entry-content">
        <div className="log-entry-main">
          <TypewriterText text={entry.text} />
          {isCommand && <CopyButton text={entry.text} />}
        </div>
        {isCommand && <EntryDescription text={entry.text} />}
      </div>
      {trace && trace.tier && trace.model && (
        <TraceAnnotation
          tier={trace.tier}
          model={trace.model}
          agentName={trace.agentName}
          confidence={trace.confidence}
          cost={trace.cost}
          compact
        />
      )}
      <div className="log-entry-detail">
        {entryType === 'error' ? 'Error encountered' : 'Executed'}
      </div>
    </div>
  );
}

/* ── Scroll shadow hook ──────────────────────────────── */

function useScrollShadow(ref: React.RefObject<HTMLDivElement | null>) {
  const [showTopShadow, setShowTopShadow] = useState(false);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    const handleScroll = () => {
      setShowTopShadow(el.scrollTop > 8);
    };

    el.addEventListener('scroll', handleScroll, { passive: true });
    handleScroll();
    return () => el.removeEventListener('scroll', handleScroll);
  }, [ref]);

  return showTopShadow;
}

/* ── Main component ──────────────────────────────────── */

export default function CommandLog({ entries, maxHeight = '300px' }: CommandLogProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const prevLengthRef = useRef(0);
  const [exitingSet, setExitingSet] = useState<Set<number>>(new Set());

  const showTopShadow = useScrollShadow(containerRef);

  // Track the newest entry index for glow highlight
  const newestIndex = entries.length - 1;

  // Smooth scroll to newest entry
  useEffect(() => {
    if (entries.length > prevLengthRef.current) {
      bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
    prevLengthRef.current = entries.length;
  }, [entries.length]);

  // Cascade-out animation when entries are cleared
  useEffect(() => {
    if (prevLengthRef.current > 0 && entries.length === 0) {
      // Already cleared, nothing to animate
      setExitingSet(new Set());
    }
  }, [entries.length]);

  // Build grouped render items
  const renderItems = useMemo(() => groupEntries(entries), [entries]);

  return (
    <div className={`command-log-wrapper${showTopShadow ? ' has-scroll-shadow' : ''}`}>
      <div className="command-log" ref={containerRef} style={{ maxHeight }}>
        {renderItems.map((item) => {
          if (item.kind === 'group') {
            return (
              <LogGroup
                key={`g-${item.entries[0].index}`}
                group={item}
                newestIndex={newestIndex}
                exitingSet={exitingSet}
              />
            );
          }

          return (
            <LogEntryRow
              key={item.index}
              entry={item.entry}
              index={item.index}
              isNewest={item.index === newestIndex}
              isExiting={exitingSet.has(item.index)}
            />
          );
        })}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
