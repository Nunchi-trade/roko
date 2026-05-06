import { useEffect, useRef, useState } from 'react';
import './SlotNumber.css';

interface SlotNumberProps {
  value: number;
  format?: (n: number) => string;
  duration?: number;  // ms, default 600
  className?: string;
}

interface CharState {
  char: string;
  key: number;
  animating: boolean;
}

let charKeyCounter = 0;

export function SlotNumber({
  value,
  format = String,
  duration = 600,
  className,
}: SlotNumberProps) {
  const formatted = format(value);
  const prevFormatted = useRef(formatted);
  const [chars, setChars] = useState<CharState[]>(() =>
    formatted.split('').map((c) => ({ char: c, key: charKeyCounter++, animating: false })),
  );

  useEffect(() => {
    const prev = prevFormatted.current;
    const next = formatted;
    if (prev === next) return;
    prevFormatted.current = next;

    const nextChars = next.split('');
    const prevChars = prev.split('');

    setChars(
      nextChars.map((c, i) => {
        const changed = i >= prevChars.length || prevChars[i] !== c;
        return { char: c, key: changed ? charKeyCounter++ : chars[i]?.key ?? charKeyCounter++, animating: changed };
      }),
    );

    // Clear animating flag after duration
    const timer = setTimeout(() => {
      setChars((cs) => cs.map((c) => ({ ...c, animating: false })));
    }, duration);

    return () => clearTimeout(timer);
  }, [formatted, duration]); // eslint-disable-line react-hooks/exhaustive-deps

  const cls = ['slot-number', className].filter(Boolean).join(' ');

  return (
    <span className={cls}>
      {chars.map((c) => (
        <span key={c.key} className="slot-number__col">
          <span
            className={`slot-number__char${c.animating ? ' slot-number__char--rolling' : ''}`}
            style={c.animating ? { animationDuration: `${duration}ms` } : undefined}
          >
            {c.char}
          </span>
        </span>
      ))}
    </span>
  );
}
