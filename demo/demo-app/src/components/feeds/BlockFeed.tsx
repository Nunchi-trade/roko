import { useRef, useEffect, useState, useCallback } from 'react';
import { relativeTime } from '../../lib/format';
import './BlockFeed.css';

/* ── Types ── */

export interface Block {
  number: number;
  hash: string;
  timestamp: number;
  txCount: number;
  gasUsed?: number;
}

export interface BlockFeedProps {
  blocks: Block[];
  maxHeight?: number | string;
  autoScroll?: boolean;
  className?: string;
}

/* ── Helpers ── */

function truncateHash(hash: string): string {
  if (hash.length <= 10) return hash;
  return `${hash.slice(0, 6)}\u2026${hash.slice(-4)}`;
}

function formatGas(gas: number): string {
  if (gas >= 1_000_000) return `${(gas / 1_000_000).toFixed(1)}M`;
  if (gas >= 1_000) return `${(gas / 1_000).toFixed(0)}K`;
  return String(gas);
}

const SCROLL_THRESHOLD = 50;

/* ── Component ── */

export default function BlockFeed({
  blocks,
  maxHeight = '280px',
  autoScroll = true,
  className,
}: BlockFeedProps) {
  const bodyRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const [pinned, setPinned] = useState(true);
  const prevLenRef = useRef(blocks.length);

  /* Track scroll position */
  const handleScroll = useCallback(() => {
    const el = bodyRef.current;
    if (!el) return;
    const nearBottom =
      el.scrollHeight - el.scrollTop - el.clientHeight < SCROLL_THRESHOLD;
    setPinned(nearBottom);
  }, []);

  /* Auto-scroll on new blocks */
  useEffect(() => {
    if (!autoScroll) return;
    const added = blocks.length - prevLenRef.current;
    prevLenRef.current = blocks.length;
    if (added > 0 && pinned) {
      bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [blocks.length, pinned, autoScroll]);

  return (
    <div
      className={`block-feed${className ? ` ${className}` : ''}`}
      style={{ maxHeight }}
    >
      <div
        ref={bodyRef}
        className="block-feed__body"
        onScroll={handleScroll}
        style={{ maxHeight }}
      >
        {blocks.length === 0 ? (
          <div className="block-feed__empty">No blocks</div>
        ) : (
          <div className="block-feed__list">
            {blocks.map((block, i) => (
              <div
                key={block.number}
                className={`block-feed__row${i === 0 ? ' block-feed__row--new' : ''}`}
                style={{ animationDelay: `${Math.min(i, 6) * 30}ms` }}
              >
                <span className="block-feed__node" />
                <span className="block-feed__num">
                  #{block.number.toLocaleString()}
                </span>
                <span className="block-feed__hash">
                  {truncateHash(block.hash)}
                </span>
                <span className="block-feed__tx">
                  {block.txCount} tx{block.txCount !== 1 ? 's' : ''}
                </span>
                {block.gasUsed != null && (
                  <span className="block-feed__gas">
                    {formatGas(block.gasUsed)} gas
                  </span>
                )}
                <span className="block-feed__ago">
                  {relativeTime(block.timestamp)}
                </span>
              </div>
            ))}
          </div>
        )}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
