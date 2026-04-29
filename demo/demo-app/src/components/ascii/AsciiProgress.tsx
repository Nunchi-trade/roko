import { useMemo } from 'react';
import './AsciiProgress.css';

interface AsciiProgressProps {
  value: number;
  width?: number;
  variant?: 'blocks' | 'braille' | 'arrows';
  label?: string;
  showPercent?: boolean;
  color?: string;
  className?: string;
}

const FILLED: Record<string, string> = {
  blocks:  '\u2588', // █
  braille: '\u28FF', // ⣿
  arrows:  '\u25B8', // ▸
};

const EMPTY: Record<string, string> = {
  blocks:  '\u2591', // ░
  braille: '\u2800', // ⠀ (blank braille)
  arrows:  '\u25B9', // ▹
};

// For braille, intermediate density chars for the boundary cell
const BRAILLE_GRADIENT = [
  '\u2800', // ⠀ 0 dots
  '\u2801', // ⠁
  '\u2803', // ⠃
  '\u2807', // ⠇
  '\u2847', // ⡇
  '\u28C7', // ⣇
  '\u28E7', // ⣧
  '\u28F7', // ⣷
  '\u28FF', // ⣿ 8 dots
];

export function AsciiProgress({
  value,
  width = 20,
  variant = 'blocks',
  label,
  showPercent = false,
  color,
  className,
}: AsciiProgressProps) {
  const clamped = Math.max(0, Math.min(1, value));

  const { filled, empty } = useMemo(() => {
    const filledCount = Math.floor(clamped * width);
    const remainder = clamped * width - filledCount;
    const emptyCount = width - filledCount - (remainder > 0 ? 1 : 0);

    const filledChar = FILLED[variant] ?? FILLED.blocks;
    const emptyChar = EMPTY[variant] ?? EMPTY.blocks;

    let filledStr = filledChar.repeat(filledCount);

    // Add partial cell for braille variant
    if (variant === 'braille' && remainder > 0) {
      const gradientIdx = Math.round(remainder * (BRAILLE_GRADIENT.length - 1));
      filledStr += BRAILLE_GRADIENT[gradientIdx];
    } else if (remainder > 0) {
      // For blocks/arrows, just snap to filled or empty
      if (remainder >= 0.5) {
        filledStr += filledChar;
      } else {
        return { filled: filledStr, empty: emptyChar.repeat(emptyCount + 1) };
      }
    }

    const actualEmpty = width - [...filledStr].length;
    return { filled: filledStr, empty: emptyChar.repeat(Math.max(0, actualEmpty)) };
  }, [clamped, width, variant]);

  const filledStyle = color ? { color } as React.CSSProperties : undefined;
  const cls = ['ascii-progress', className ?? ''].filter(Boolean).join(' ');

  return (
    <span className={cls} aria-valuenow={Math.round(clamped * 100)} aria-valuemin={0} aria-valuemax={100} role="progressbar">
      {label && <span className="ascii-progress__label">{label}</span>}
      <span className="ascii-progress__bar">
        <span className="ascii-progress__bracket">[</span>
        <span className="ascii-progress__filled" style={filledStyle}>{filled}</span>
        <span className="ascii-progress__empty">{empty}</span>
        <span className="ascii-progress__bracket">]</span>
      </span>
      {showPercent && (
        <span className="ascii-progress__percent">{Math.round(clamped * 100)}%</span>
      )}
    </span>
  );
}
