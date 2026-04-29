import { useMemo } from 'react';
import './AsciiDivider.css';

interface AsciiDividerProps {
  variant?: 'line' | 'double' | 'dashed' | 'dotted' | 'braille' | 'chevron';
  label?: string;
  color?: string;
  className?: string;
}

const REPEAT_CHARS: Record<string, string> = {
  line:    '\u2500', // ─
  double:  '\u2550', // ═
  dashed:  '\u254C', // ╌
  dotted:  '\u00B7', // ·
  braille: '\u2812', // ⠒
  chevron: '\u2571\u2572', // ╱╲
};

// Enough characters to fill any reasonable width
const FILL_COUNT = 200;

export function AsciiDivider({
  variant = 'line',
  label,
  color,
  className,
}: AsciiDividerProps) {
  const fillText = useMemo(() => {
    const ch = REPEAT_CHARS[variant] ?? REPEAT_CHARS.line;
    // For dotted, add spacing between dots
    if (variant === 'dotted') return (ch + ' ').repeat(FILL_COUNT);
    return ch.repeat(FILL_COUNT);
  }, [variant]);

  const style = color ? { color } as React.CSSProperties : undefined;

  const cls = ['ascii-divider', className ?? ''].filter(Boolean).join(' ');

  if (label) {
    return (
      <div className={cls} style={style} aria-hidden="true">
        <span className="ascii-divider__segment">{fillText}</span>
        <span className="ascii-divider__label">{label}</span>
        <span className="ascii-divider__segment">{fillText}</span>
      </div>
    );
  }

  return (
    <div className={cls} style={style} aria-hidden="true">
      <span className="ascii-divider__segment">{fillText}</span>
    </div>
  );
}
