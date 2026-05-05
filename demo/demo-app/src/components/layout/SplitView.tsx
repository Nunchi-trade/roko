import { useState, useCallback, useRef, useEffect, type ReactNode } from 'react';
import './SplitView.css';

interface SplitViewProps {
  left: ReactNode;
  right: ReactNode;
  defaultSplit?: number;
  minLeft?: number;
  maxLeft?: number;
  direction?: 'horizontal' | 'vertical';
  className?: string;
}

export function SplitView({
  left,
  right,
  defaultSplit = 40,
  minLeft = 20,
  maxLeft = 80,
  direction = 'horizontal',
  className,
}: SplitViewProps) {
  const [split, setSplit] = useState(defaultSplit);
  const dragging = useRef(false);
  const containerRef = useRef<HTMLDivElement>(null);

  const onPointerDown = useCallback((e: React.PointerEvent) => {
    e.preventDefault();
    dragging.current = true;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }, []);

  const onPointerMove = useCallback(
    (e: React.PointerEvent) => {
      if (!dragging.current || !containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      let pct: number;
      if (direction === 'horizontal') {
        pct = ((e.clientX - rect.left) / rect.width) * 100;
      } else {
        pct = ((e.clientY - rect.top) / rect.height) * 100;
      }
      setSplit(Math.min(maxLeft, Math.max(minLeft, pct)));
    },
    [direction, minLeft, maxLeft],
  );

  const onPointerUp = useCallback(() => {
    dragging.current = false;
  }, []);

  // Clean up pointer capture if component unmounts mid-drag
  useEffect(() => {
    return () => {
      dragging.current = false;
    };
  }, []);

  const isHoriz = direction === 'horizontal';
  const gridTemplate = isHoriz
    ? { gridTemplateColumns: `${split}% 1fr` }
    : { gridTemplateRows: `${split}% 1fr` };

  return (
    <div
      ref={containerRef}
      className={`split-view split-view--${direction}${className ? ` ${className}` : ''}`}
      style={gridTemplate}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
    >
      <div className="split-view__pane">{left}</div>
      <div
        className="split-view__divider"
        onPointerDown={onPointerDown}
      >
        <div className="split-view__divider-line" />
      </div>
      <div className="split-view__pane">{right}</div>
    </div>
  );
}
