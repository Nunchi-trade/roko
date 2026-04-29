import { useState, useRef, useEffect, useCallback } from 'react';
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
}

interface ChainActivityProps {
  blocks: BlockData[];
  maxBlocks?: number;
}

/* ── Helpers ── */
function relativeTime(ts: number): string {
  const delta = Math.max(0, Math.floor((Date.now() - ts) / 1000));
  if (delta < 60) return `${delta}s ago`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`;
  return `${Math.floor(delta / 3600)}h ago`;
}

function truncateHash(hash: string): string {
  if (hash.length <= 8) return hash;
  return `${hash.slice(0, 6)}..`;
}

function formatBlockNum(n: number): string {
  return n.toLocaleString();
}

/* ── Component ── */
export default function ChainActivityPanel({ blocks, maxBlocks = 20 }: ChainActivityProps) {
  const visible = blocks.slice(0, maxBlocks);
  const [expanded, setExpanded] = useState<Set<number>>(() => {
    // Auto-expand the first block
    if (visible.length > 0) return new Set([visible[0].number]);
    return new Set();
  });
  const scrollRef = useRef<HTMLDivElement>(null);

  // Auto-expand newest block when it appears
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

  const totalTx = visible.reduce((s, b) => s + b.transactions.length, 0);

  return (
    <Pane
      title="Chain Activity"
      badge={<span>{totalTx} txs</span>}
      flat
    >
      <div className="chain-activity" ref={scrollRef}>
        {visible.length === 0 ? (
          <div className="ca-empty">Waiting for blocks...</div>
        ) : (
          visible.map(block => {
            const isOpen = expanded.has(block.number);
            return (
              <div
                key={block.number}
                className={`ca-block${isOpen ? ' expanded' : ''}`}
              >
                <div className="ca-block-head" onClick={() => toggle(block.number)}>
                  <span className="ca-block-num">
                    Block {formatBlockNum(block.number)}
                  </span>
                  <div className="ca-block-meta">
                    <span className="ca-block-ago">{relativeTime(block.timestamp)}</span>
                    <span className="ca-block-count">
                      {block.transactions.length} tx{block.transactions.length !== 1 ? 's' : ''}
                    </span>
                    <span className="ca-block-chevron">{'\u25B6'}</span>
                  </div>
                </div>
                <div className="ca-txs">
                  {block.transactions.map((tx, i) => (
                    <div key={tx.hash + i} className="ca-tx" data-type={tx.type}>
                      <span className="ca-tx-dot" />
                      <span className="ca-tx-hash">{truncateHash(tx.hash)}</span>
                      <span className="ca-tx-desc">{tx.description}</span>
                    </div>
                  ))}
                </div>
              </div>
            );
          })
        )}
      </div>
    </Pane>
  );
}
