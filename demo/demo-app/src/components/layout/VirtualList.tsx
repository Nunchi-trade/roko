import {
  useState,
  useCallback,
  useRef,
  useImperativeHandle,
  forwardRef,
  type ReactNode,
  type CSSProperties,
} from 'react';

/* ── public types ── */

export interface VirtualListHandle {
  scrollToIndex: (index: number) => void;
}

export interface VirtualListProps<T> {
  items: T[];
  itemHeight: number;
  overscan?: number;
  renderItem: (item: T, index: number) => ReactNode;
  className?: string;
  style?: CSSProperties;
}

/* ── component ── */

function VirtualListInner<T>(
  {
    items,
    itemHeight,
    overscan = 5,
    renderItem,
    className,
    style,
  }: VirtualListProps<T>,
  ref: React.Ref<VirtualListHandle>,
) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [containerHeight, setContainerHeight] = useState(0);

  // Measure container on first render and resize
  const measureRef = useCallback((el: HTMLDivElement | null) => {
    if (!el) return;
    (containerRef as React.MutableRefObject<HTMLDivElement | null>).current = el;
    setContainerHeight(el.clientHeight);

    // Watch for resize via ResizeObserver
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        setContainerHeight(entry.contentRect.height);
      }
    });
    observer.observe(el);

    // Cleanup handled by React — observer persists for component lifetime
    return () => observer.disconnect();
  }, []);

  // Imperative scroll-to
  useImperativeHandle(ref, () => ({
    scrollToIndex: (index: number) => {
      containerRef.current?.scrollTo({
        top: index * itemHeight,
        behavior: 'smooth',
      });
    },
  }));

  const handleScroll = useCallback(() => {
    if (containerRef.current) {
      setScrollTop(containerRef.current.scrollTop);
    }
  }, []);

  // Calculate visible range
  const totalHeight = items.length * itemHeight;
  const startIndex = Math.max(0, Math.floor(scrollTop / itemHeight) - overscan);
  const visibleCount = Math.ceil(containerHeight / itemHeight);
  const endIndex = Math.min(items.length - 1, startIndex + visibleCount + overscan * 2);

  // Build visible items
  const visibleItems: ReactNode[] = [];
  for (let i = startIndex; i <= endIndex; i++) {
    visibleItems.push(
      <div
        key={i}
        style={{
          position: 'absolute',
          top: i * itemHeight,
          left: 0,
          right: 0,
          height: itemHeight,
        }}
      >
        {renderItem(items[i], i)}
      </div>,
    );
  }

  return (
    <div
      ref={measureRef}
      className={className}
      style={{
        overflowY: 'auto',
        position: 'relative',
        ...style,
      }}
      onScroll={handleScroll}
    >
      <div
        style={{
          height: totalHeight,
          position: 'relative',
          width: '100%',
        }}
      >
        {visibleItems}
      </div>
    </div>
  );
}

// Wrap with forwardRef while preserving generic
const VirtualList = forwardRef(VirtualListInner) as <T>(
  props: VirtualListProps<T> & { ref?: React.Ref<VirtualListHandle> },
) => React.ReactElement | null;

export default VirtualList;
