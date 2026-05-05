import './Skeleton.css';

interface SkeletonProps {
  variant?: 'text' | 'rect' | 'circle' | 'pane';
  width?: string | number;
  height?: string | number;
  lines?: number;
}

function toPx(value: string | number): string {
  return typeof value === 'number' ? `${value}px` : value;
}

export function Skeleton({
  variant = 'rect',
  width,
  height,
  lines,
}: SkeletonProps) {
  // Text with multiple lines
  if (variant === 'text' && lines && lines > 1) {
    return (
      <div className="skeleton-lines">
        {Array.from({ length: lines }, (_, i) => (
          <div
            key={i}
            className="skeleton skeleton--text"
            style={{
              width: width ? toPx(width) : undefined,
              height: height ? toPx(height) : undefined,
            }}
          />
        ))}
      </div>
    );
  }

  const style: React.CSSProperties = {};
  if (width) style.width = toPx(width);
  if (height) style.height = toPx(height);

  // Circle defaults to equal width/height
  if (variant === 'circle') {
    const size = width ?? height ?? 32;
    style.width = toPx(size);
    style.height = toPx(size);
  }

  return <div className={`skeleton skeleton--${variant}`} style={style} />;
}
