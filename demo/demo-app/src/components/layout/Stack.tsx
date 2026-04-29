import type { CSSProperties, ReactNode } from 'react';

interface StackProps {
  direction?: 'vertical' | 'horizontal';
  gap?: string;
  align?: 'start' | 'center' | 'end' | 'stretch';
  justify?: 'start' | 'center' | 'end' | 'between' | 'around';
  wrap?: boolean;
  className?: string;
  style?: CSSProperties;
  children: ReactNode;
}

const JUSTIFY_MAP: Record<string, string> = {
  start: 'flex-start',
  center: 'center',
  end: 'flex-end',
  between: 'space-between',
  around: 'space-around',
};

const ALIGN_MAP: Record<string, string> = {
  start: 'flex-start',
  center: 'center',
  end: 'flex-end',
  stretch: 'stretch',
};

export function Stack({
  direction = 'vertical',
  gap = 'var(--sp-4)',
  align,
  justify,
  wrap,
  className,
  style,
  children,
}: StackProps) {
  const stackStyle: CSSProperties = {
    display: 'flex',
    flexDirection: direction === 'vertical' ? 'column' : 'row',
    gap,
    ...(align && { alignItems: ALIGN_MAP[align] }),
    ...(justify && { justifyContent: JUSTIFY_MAP[justify] }),
    ...(wrap && { flexWrap: 'wrap' }),
    ...style,
  };

  return (
    <div className={className} style={stackStyle}>
      {children}
    </div>
  );
}
