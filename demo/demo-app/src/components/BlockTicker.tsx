import { useRef, useEffect } from 'react';
import { useBlockStream } from '../hooks/useBlockStream';
import './BlockTicker.css';

function truncateHash(hash: string): string {
  if (!hash || hash.length < 12) return hash;
  return `${hash.slice(0, 8)}...${hash.slice(-4)}`;
}

export default function BlockTicker() {
  const { blocks, connected } = useBlockStream();
  const scrollRef = useRef<HTMLDivElement>(null);
  const prevCountRef = useRef(0);

  // Auto-scroll to right when new blocks arrive
  useEffect(() => {
    if (blocks.length > prevCountRef.current && scrollRef.current) {
      scrollRef.current.scrollLeft = scrollRef.current.scrollWidth;
    }
    prevCountRef.current = blocks.length;
  }, [blocks.length]);

  if (!connected || blocks.length === 0) return null;

  return (
    <div className="block-ticker">
      <span className="block-ticker-label">chain</span>
      <div className="block-ticker-scroll" ref={scrollRef}>
        {blocks.map((b, i) => (
          <span
            key={b.number}
            className={`block-ticker-item${i === blocks.length - 1 ? ' latest' : ''}`}
          >
            <span className="block-ticker-num">#{b.number}</span>
            <span className="block-ticker-hash">{truncateHash(b.hash)}</span>
          </span>
        ))}
      </div>
    </div>
  );
}
