import { useEffect, useRef, useState, useCallback } from 'react';
import './AsciiBraille.css';

interface AsciiBrailleProps {
  pattern?: 'noise' | 'wave' | 'density' | 'spinner';
  width?: number;
  height?: number;
  density?: number;
  speed?: number;
  color?: string;
  className?: string;
}

// Full braille range: U+2800 to U+28FF (256 characters)
const BRAILLE_BASE = 0x2800;
const BRAILLE_COUNT = 256;

// Spinner frames
const SPINNER_FRAMES = [
  '\u280B', // ⠋
  '\u2819', // ⠙
  '\u2839', // ⠹
  '\u2838', // ⠸
  '\u283C', // ⠼
  '\u2834', // ⠴
  '\u2826', // ⠦
  '\u2827', // ⠧
  '\u2807', // ⠇
  '\u280F', // ⠏
];

function randomBraille(): string {
  return String.fromCharCode(BRAILLE_BASE + Math.floor(Math.random() * BRAILLE_COUNT));
}

// Map density (0-1) to braille by number of dots set
function densityBraille(d: number): string {
  // Set roughly d*8 dots (braille has 8 dot positions)
  const dotsToSet = Math.round(d * 8);
  // Each bit in the braille offset represents a dot
  let code = 0;
  const positions = [0, 1, 2, 3, 4, 5, 6, 7];
  // Shuffle positions
  for (let i = positions.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [positions[i], positions[j]] = [positions[j], positions[i]];
  }
  for (let i = 0; i < dotsToSet; i++) {
    code |= (1 << positions[i]);
  }
  return String.fromCharCode(BRAILLE_BASE + code);
}

function generateNoiseGrid(w: number, h: number): string[] {
  const lines: string[] = [];
  for (let row = 0; row < h; row++) {
    let line = '';
    for (let col = 0; col < w; col++) {
      line += randomBraille();
    }
    lines.push(line);
  }
  return lines;
}

function generateWaveGrid(w: number, h: number, phase: number): string[] {
  const lines: string[] = [];
  for (let row = 0; row < h; row++) {
    let line = '';
    for (let col = 0; col < w; col++) {
      // Sine wave density moving left to right
      const d = (Math.sin((col / w) * Math.PI * 2 + phase + row * 0.3) + 1) / 2;
      line += densityBraille(d);
    }
    lines.push(line);
  }
  return lines;
}

function generateDensityGrid(w: number, h: number, d: number): string[] {
  const lines: string[] = [];
  for (let row = 0; row < h; row++) {
    let line = '';
    for (let col = 0; col < w; col++) {
      line += densityBraille(d);
    }
    lines.push(line);
  }
  return lines;
}

export function AsciiBraille({
  pattern = 'noise',
  width = 20,
  height = 3,
  density: densityProp = 0.5,
  speed = 1,
  color,
  className,
}: AsciiBrailleProps) {
  const [content, setContent] = useState('');
  const rafRef = useRef(0);
  const lastTickRef = useRef(0);
  const phaseRef = useRef(0);

  const baseInterval = pattern === 'spinner' ? 80 : 100;
  const interval = baseInterval / speed;

  const tick = useCallback((time: number) => {
    if (lastTickRef.current === 0) lastTickRef.current = time;
    const elapsed = time - lastTickRef.current;

    if (elapsed >= interval) {
      lastTickRef.current = time;

      if (pattern === 'spinner') {
        phaseRef.current = (phaseRef.current + 1) % SPINNER_FRAMES.length;
        setContent(SPINNER_FRAMES[phaseRef.current]);
      } else if (pattern === 'noise') {
        // Only change ~20% of characters per tick for subtlety
        setContent(prev => {
          const lines = prev ? prev.split('\n') : generateNoiseGrid(width, height);
          const next = lines.map(line => {
            const chars = [...line];
            for (let i = 0; i < chars.length; i++) {
              if (Math.random() < 0.2) {
                chars[i] = randomBraille();
              }
            }
            return chars.join('');
          });
          return next.join('\n');
        });
      } else if (pattern === 'wave') {
        phaseRef.current += 0.15;
        setContent(generateWaveGrid(width, height, phaseRef.current).join('\n'));
      }
    }

    rafRef.current = requestAnimationFrame(tick);
  }, [pattern, width, height, interval]);

  useEffect(() => {
    // Initialize content
    if (pattern === 'density') {
      setContent(generateDensityGrid(width, height, densityProp).join('\n'));
      return; // Static, no animation
    }
    if (pattern === 'spinner') {
      setContent(SPINNER_FRAMES[0]);
    } else if (pattern === 'noise') {
      setContent(generateNoiseGrid(width, height).join('\n'));
    } else if (pattern === 'wave') {
      setContent(generateWaveGrid(width, height, 0).join('\n'));
    }

    lastTickRef.current = 0;
    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
  }, [pattern, width, height, densityProp, tick]);

  const style = color ? { color } as React.CSSProperties : undefined;
  const cls = [
    'ascii-braille',
    pattern === 'spinner' ? 'ascii-braille--spinner' : '',
    className ?? '',
  ].filter(Boolean).join(' ');

  return (
    <span className={cls} style={style} aria-hidden="true">
      {content}
    </span>
  );
}
