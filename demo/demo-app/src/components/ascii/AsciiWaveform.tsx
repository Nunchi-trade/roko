import { useMemo } from 'react';
import './AsciiWaveform.css';

interface AsciiWaveformProps {
  values: number[];
  width?: number;
  height?: number;
  color?: string;
  label?: string;
  className?: string;
}

// 8 block levels: ▁▂▃▄▅▆▇█
const BLOCKS = [
  '\u2581', // ▁
  '\u2582', // ▂
  '\u2583', // ▃
  '\u2584', // ▄
  '\u2585', // ▅
  '\u2586', // ▆
  '\u2587', // ▇
  '\u2588', // █
];

const SPACE = ' ';

export function AsciiWaveform({
  values,
  width,
  height = 1,
  color,
  label,
  className,
}: AsciiWaveformProps) {
  const displayWidth = width ?? values.length;

  const grid = useMemo(() => {
    // Resample values to fit displayWidth
    const sampled: number[] = [];
    for (let i = 0; i < displayWidth; i++) {
      if (values.length === 0) {
        sampled.push(0);
        continue;
      }
      const srcIdx = (i / displayWidth) * values.length;
      const lo = Math.floor(srcIdx);
      const hi = Math.min(lo + 1, values.length - 1);
      const t = srcIdx - lo;
      const v = values[lo] * (1 - t) + values[hi] * t;
      sampled.push(Math.max(0, Math.min(1, v)));
    }

    if (height === 1) {
      // Simple single-row: map each value to a block character
      return [sampled.map(v => {
        const idx = Math.round(v * (BLOCKS.length - 1));
        return BLOCKS[idx];
      }).join('')];
    }

    // Multi-row: build from bottom up
    // Each row covers 1/height of the range
    const rows: string[] = [];
    for (let row = height - 1; row >= 0; row--) {
      const rowMin = row / height;
      const rowMax = (row + 1) / height;
      let line = '';

      for (let col = 0; col < displayWidth; col++) {
        const v = sampled[col];
        if (v >= rowMax) {
          // Full block
          line += BLOCKS[BLOCKS.length - 1];
        } else if (v > rowMin) {
          // Partial block
          const frac = (v - rowMin) / (rowMax - rowMin);
          const idx = Math.round(frac * (BLOCKS.length - 1));
          line += BLOCKS[idx];
        } else {
          line += SPACE;
        }
      }

      rows.push(line);
    }

    return rows;
  }, [values, displayWidth, height]);

  const style = color ? { color } as React.CSSProperties : undefined;
  const cls = ['ascii-waveform', className ?? ''].filter(Boolean).join(' ');

  return (
    <span className={cls} aria-hidden="true">
      {label && <span className="ascii-waveform__label">{label}</span>}
      <span className="ascii-waveform__bars" style={style}>
        {grid.map((line, i) => (
          <span key={i} className="ascii-waveform__bar">
            {line}
            {i < grid.length - 1 ? '\n' : ''}
          </span>
        ))}
      </span>
    </span>
  );
}
