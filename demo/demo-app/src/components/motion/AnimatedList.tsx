import type { ReactNode } from 'react';
import './AnimatedList.css';

interface AnimatedListProps<T> {
  items: T[];
  keyFn: (item: T) => string;
  renderItem: (item: T, index: number) => ReactNode;
  className?: string;
}

export function AnimatedList<T>({
  items,
  keyFn,
  renderItem,
  className,
}: AnimatedListProps<T>) {
  return (
    <div className={className}>
      {items.map((item, i) => (
        <div
          key={keyFn(item)}
          className="animated-list-item"
          style={{ '--i': i } as React.CSSProperties}
        >
          {renderItem(item, i)}
        </div>
      ))}
    </div>
  );
}
