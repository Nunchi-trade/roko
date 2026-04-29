import { useState, useRef, useEffect, useCallback, useMemo } from 'react';
import { handleRowKeyDown } from '../lib/a11y';
import Pane from './Pane';
import './ChainActivityPanel.css';

/* ── Types ── */
export interface TxData {
  hash: string;
  type: 'read' | 'insight' | 'defi' | 'other';
  description: string;
}

export interface BlockData {
  number: number;
  timestamp: number;
  transactions: TxData[];
  status?: 'confirmed' | 'pending';
}

interface ChainActivityProps {
  blocks: BlockData[];
  maxBlocks?: number;
}

/* ── Helpers ── */
function truncateHash(hash: string): string {
  if (hash.length <= 8) return hash;
  return `${hash.slice(0, 6)}..`;
}

function formatBlockNum(n: number): string {
  return n.toLocaleString();
}

/* ── Live relative time hook ── */
function useRelativeTime(ms: number): string {
  const [, setTick] = useState(0);

  useEffect(() => {
    const id = setInterval(() => setTick(t => t + 1), 1000);
    return () => clearInterval(id);
  }, []);

  const delta = Math.max(0, Date.now() - ms);
  if (delta < 60_000) return `${Math.floor(delta / 1000)}s ago`;
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m ago`;
  if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h ago`;
  return new Date(ms).toLocaleDateString();
}

/* ── Typewriter hash reveal ── */
function TypewriterHash({ hash, delay = 0 }: { hash: string; delay?: number }) {
  const [revealed, setRevealed] = useState(0);
  const full = truncateHash(hash);

  useEffect(() => {
    const timeout = setTimeout(() => {
      const id = setInterval(() => {
        setRevealed(prev => {
          if (prev >= full.length) {
            clearInterval(id);
            return prev;
          }
          return prev + 1;
        });
      }, 30);
      return () => clearInterval(id);
    }, delay);
    return () => clearTimeout(timeout);
  }, [full, delay]);

  return (
    <span className="ca-tx-hash ca-hash-typewriter">
      {full.split('').map((ch, i) => (
        <span
          key={i}
          className={`ca-hash-char ${i < revealed ? 'revealed' : ''} ${i === revealed - 1 ? 'flash' : ''}`}
        >
          {i < revealed ? ch : '\u00B7'}
        </span>
      ))}
    </span>
  );
}

/* ── Status badge ── */
function StatusBadge({ status }: { status: 'confirmed' | 'pending' }) {
  return (
    <span className={`ca-status-badge ca-status-${status}`}>
      <span className="ca-status-dot" />
      {status}
    </span>
  );
}

/* ── Block timestamp (live-updating) ── */
function BlockTimestamp({ timestamp }: { timestamp: number }) {
  const text = useRelativeTime(timestamp);
  return <span className="ca-block-ago">{text}</span>;
}

/* ── Block row with hover expand ── */
function BlockRow({
  block,
  isOpen,
  isNew,
  onToggle,
}: {
  block: BlockData;
  isOpen: boolean;
  isNew: boolean;
  onToggle: () => void;
}) {
  const [hovered, setHovered] = useState(false);
  const status = block.status ?? 'confirmed';

  return (
    <div
      className={[
        'ca-block',
        isOpen ? 'expanded' : '',
        isNew ? 'ca-block-new' : '',
        hovered ? 'ca-block-hovered' : '',
      ].filter(Boolean).join(' ')}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      {/* Chain connector line with pulse */}
      <div className="ca-chain-connector">
        <div className="ca-chain-line" />
        <div className="ca-chain-node" />
        <div className="ca-chain-pulse" />
      </div>

      <div className="ca-block-content">
        <div
          className="ca-block-head"
          tabIndex={0}
          role="button"
          onClick={onToggle}
          onKeyDown={(e) => handleRowKeyDown(e, onToggle)}
        >
          <div className="ca-block-left">
            <span className="ca-block-num">
              {isNew && <span className="ca-block-new-indicator" />}
              Block {formatBlockNum(block.number)}
            </span>
            <StatusBadge status={status} />
          </div>
          <div className="ca-block-meta">
            <BlockTimestamp timestamp={block.timestamp} />
            <span className="ca-block-count">
              {block.transactions.length} tx{block.transactions.length !== 1 ? 's' : ''}
            </span>
            <span className="ca-block-chevron">{'\u25B6'}</span>
          </div>
        </div>

        {/* Hover expand: block detail summary */}
        <div className="ca-block-hover-detail">
          <div className="ca-hover-row">
            <span className="ca-hover-label">Block</span>
            <span className="ca-hover-value">#{formatBlockNum(block.number)}</span>
          </div>
          <div className="ca-hover-row">
            <span className="ca-hover-label">Txns</span>
            <span className="ca-hover-value">{block.transactions.length}</span>
          </div>
          <div className="ca-hover-row">
            <span className="ca-hover-label">Status</span>
            <StatusBadge status={status} />
          </div>
        </div>

        <div className="ca-txs">
          {block.transactions.map((tx, i) => (
            <div key={tx.hash + i} className="ca-tx" data-type={tx.type}>
              <span className="ca-tx-dot" />
              <TypewriterHash hash={tx.hash} delay={i * 60} />
              <span className="ca-tx-desc">{tx.description}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

/* ── Component ── */
export default function ChainActivityPanel({ blocks, maxBlocks = 20 }: ChainActivityProps) {
  const visible = blocks.slice(0, maxBlocks);
  const [expanded, setExpanded] = useState<Set<number>>(() => {
    if (visible.length > 0) return new Set([visible[0].number]);
    return new Set();
  });
  const scrollRef = useRef<HTMLDivElement>(null);
  const [newBlocks, setNewBlocks] = useState<Set<number>>(new Set());

  // Track newest block and mark as "new" for entrance animation
  const prevTopRef = useRef<number | null>(null);
  useEffect(() => {
    if (visible.length === 0) return;
    const newest = visible[0].number;
    if (prevTopRef.current !== null && newest !== prevTopRef.current) {
      setExpanded(prev => {
        const next = new Set(prev);
        next.add(newest);
        return next;
      });
      setNewBlocks(prev => {
        const next = new Set(prev);
        next.add(newest);
        return next;
      });
      // Clear "new" state after animation
      const timer = setTimeout(() => {
        setNewBlocks(prev => {
          const next = new Set(prev);
          next.delete(newest);
          return next;
        });
      }, 1200);
      prevTopRef.current = newest;
      return () => clearTimeout(timer);
    }
    prevTopRef.current = newest;
  }, [visible]);

  const toggle = useCallback((blockNum: number) => {
    setExpanded(prev => {
      const next = new Set(prev);
      if (next.has(blockNum)) next.delete(blockNum);
      else next.add(blockNum);
      return next;
    });
  }, []);

  const totalTx = useMemo(
    () => visible.reduce((s, b) => s + b.transactions.length, 0),
    [visible],
  );

  return (
    <Pane
      title="Chain Activity"
      badge={<span>{totalTx} txs</span>}
      flat
    >
      <div className="chain-activity" ref={scrollRef}>
        {/* Chain integrity pulse overlay */}
        <div className="ca-integrity-pulse" />

        {visible.length === 0 ? (
          <div className="ca-empty">
            <span className="ca-empty-dot" />
            Waiting for blocks...
          </div>
        ) : (
          visible.map(block => (
            <BlockRow
              key={block.number}
              block={block}
              isOpen={expanded.has(block.number)}
              isNew={newBlocks.has(block.number)}
              onToggle={() => toggle(block.number)}
            />
          ))
        )}
      </div>
    </Pane>
  );
}
