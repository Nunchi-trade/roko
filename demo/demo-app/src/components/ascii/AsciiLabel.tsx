import { useEffect, useRef, useState, useCallback } from 'react';
import './AsciiLabel.css';

interface AsciiLabelProps {
  children: string;
  frame?: 'brackets' | 'angles' | 'pipes' | 'corners' | 'none';
  size?: 'xs' | 'sm' | 'md' | 'lg';
  color?: string;
  glow?: boolean;
  animate?: 'none' | 'typewriter' | 'flicker';
  className?: string;
}

const FRAME_CHARS: Record<string, [string, string]> = {
  brackets: ['[', ']'],
  angles:   ['\u27E8', '\u27E9'],   // ⟨ ⟩
  pipes:    ['\u2502', '\u2502'],   // │ │
  corners:  ['\u2308', '\u230B'],   // ⌈ ⌋
};

const GLITCH_CHARS = '\u2591\u2592\u2593'; // ░▒▓

export function AsciiLabel({
  children,
  frame = 'none',
  size = 'sm',
  color,
  glow = false,
  animate = 'none',
  className,
}: AsciiLabelProps) {
  const cls = [
    'ascii-label',
    `ascii-label--${size}`,
    glow ? 'ascii-label--glow' : '',
    className ?? '',
  ].filter(Boolean).join(' ');

  const style = color ? { color } as React.CSSProperties : undefined;

  const frameL = frame !== 'none' && FRAME_CHARS[frame] ? FRAME_CHARS[frame][0] : null;
  const frameR = frame !== 'none' && FRAME_CHARS[frame] ? FRAME_CHARS[frame][1] : null;

  if (animate === 'typewriter') {
    return (
      <span className={cls} style={style} aria-label={children}>
        {frameL && <span className="ascii-label__frame ascii-label__frame--left">{frameL}</span>}
        <TypewriterText text={children} />
        {frameR && <span className="ascii-label__frame ascii-label__frame--right">{frameR}</span>}
      </span>
    );
  }

  if (animate === 'flicker') {
    return (
      <span className={cls} style={style} aria-label={children}>
        {frameL && <span className="ascii-label__frame ascii-label__frame--left">{frameL}</span>}
        <FlickerText text={children} />
        {frameR && <span className="ascii-label__frame ascii-label__frame--right">{frameR}</span>}
      </span>
    );
  }

  return (
    <span className={cls} style={style}>
      {frameL && <span className="ascii-label__frame ascii-label__frame--left">{frameL}</span>}
      {children}
      {frameR && <span className="ascii-label__frame ascii-label__frame--right">{frameR}</span>}
    </span>
  );
}

/* ── Typewriter sub-component ── */

function TypewriterText({ text }: { text: string }) {
  const [visibleCount, setVisibleCount] = useState(0);
  const [showCursor, setShowCursor] = useState(true);
  const rafRef = useRef(0);
  const lastTickRef = useRef(0);

  useEffect(() => {
    setVisibleCount(0);
    setShowCursor(true);
    lastTickRef.current = 0;

    const step = (time: number) => {
      if (lastTickRef.current === 0) lastTickRef.current = time;
      const elapsed = time - lastTickRef.current;
      // ~100ms interval per character for terminal feel
      if (elapsed >= 100) {
        lastTickRef.current = time;
        setVisibleCount(prev => {
          if (prev >= text.length) {
            setShowCursor(false);
            return prev;
          }
          return prev + 1;
        });
      }
      rafRef.current = requestAnimationFrame(step);
    };

    rafRef.current = requestAnimationFrame(step);
    return () => cancelAnimationFrame(rafRef.current);
  }, [text]);

  return (
    <span className="ascii-label__typewriter">
      {text.split('').map((ch, i) => (
        <span
          key={i}
          className="ascii-label__typewriter-char"
          style={{
            animationDelay: `${i * 100}ms`,
            visibility: i < visibleCount ? 'visible' : 'hidden',
          }}
        >
          {ch}
        </span>
      ))}
      {showCursor && <span className="ascii-label__cursor">_</span>}
    </span>
  );
}

/* ── Flicker sub-component ── */

function FlickerText({ text }: { text: string }) {
  const [chars, setChars] = useState<string[]>(() => text.split(''));
  const settledRef = useRef(new Set<number>());
  const rafRef = useRef(0);
  const lastTickRef = useRef(0);

  const tick = useCallback((time: number) => {
    if (lastTickRef.current === 0) lastTickRef.current = time;
    const elapsed = time - lastTickRef.current;

    // ~100ms between flicker frames
    if (elapsed >= 100) {
      lastTickRef.current = time;

      setChars(prev => {
        const next = [...prev];
        let allSettled = true;

        for (let i = 0; i < text.length; i++) {
          if (settledRef.current.has(i)) continue;
          allSettled = false;

          // 30% chance to settle each tick
          if (Math.random() < 0.3) {
            next[i] = text[i];
            settledRef.current.add(i);
          } else {
            next[i] = GLITCH_CHARS[Math.floor(Math.random() * GLITCH_CHARS.length)];
          }
        }

        if (allSettled) return text.split('');
        return next;
      });
    }

    if (settledRef.current.size < text.length) {
      rafRef.current = requestAnimationFrame(tick);
    }
  }, [text]);

  useEffect(() => {
    settledRef.current.clear();
    setChars(text.split('').map(() => GLITCH_CHARS[Math.floor(Math.random() * GLITCH_CHARS.length)]));
    lastTickRef.current = 0;
    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
  }, [text, tick]);

  return (
    <>
      {chars.map((ch, i) => (
        <span
          key={i}
          className={`ascii-label__flicker-char${
            !settledRef.current.has(i) ? ' ascii-label__flicker-char--glitching' : ''
          }`}
        >
          {ch}
        </span>
      ))}
    </>
  );
}
