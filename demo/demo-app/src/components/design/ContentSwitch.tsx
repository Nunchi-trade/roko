import {
  useRef,
  useState,
  useEffect,
  useCallback,
  type ReactNode,
} from 'react';
import { Skeleton } from './Skeleton';
import './ContentSwitch.css';

type SwitchMode = 'crossfade' | 'fade-through';

interface ContentSwitchProps {
  contentKey: string;
  children: ReactNode;
  duration?: number;
  skeleton?: ReactNode;
  mode?: SwitchMode;
  className?: string;
}

type Phase = 'idle' | 'exit' | 'skeleton' | 'enter';

export default function ContentSwitch({
  contentKey,
  children,
  duration = 200,
  skeleton,
  mode = 'fade-through',
  className,
}: ContentSwitchProps) {
  const prevKeyRef = useRef(contentKey);
  const containerRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  const [phase, setPhase] = useState<Phase>('idle');
  const [displayedChildren, setDisplayedChildren] = useState<ReactNode>(children);
  const [height, setHeight] = useState<number | undefined>(undefined);

  const reducedMotion =
    typeof window !== 'undefined' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  // Measure current content height
  const measureHeight = useCallback(() => {
    if (contentRef.current) {
      setHeight(contentRef.current.offsetHeight);
    }
  }, []);

  useEffect(() => {
    if (contentKey === prevKeyRef.current) {
      // Key unchanged -- just update children in place
      setDisplayedChildren(children);
      return;
    }

    prevKeyRef.current = contentKey;

    if (reducedMotion) {
      setDisplayedChildren(children);
      return;
    }

    // Snapshot current height before transition
    measureHeight();

    if (mode === 'crossfade') {
      // Crossfade: exit old + enter new simultaneously
      setPhase('exit');
      clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => {
        setDisplayedChildren(children);
        setPhase('enter');
        timerRef.current = setTimeout(() => {
          setPhase('idle');
          setHeight(undefined);
        }, duration);
      }, duration);
    } else {
      // Fade-through: exit -> skeleton -> enter
      setPhase('exit');
      clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => {
        setPhase('skeleton');
        timerRef.current = setTimeout(() => {
          setDisplayedChildren(children);
          setPhase('enter');
          timerRef.current = setTimeout(() => {
            setPhase('idle');
            setHeight(undefined);
          }, duration);
        }, duration);
      }, duration);
    }
  }, [contentKey, children, duration, mode, measureHeight, reducedMotion]);

  // Update height when new content renders
  useEffect(() => {
    if (phase === 'enter' || phase === 'idle') {
      measureHeight();
    }
  }, [phase, displayedChildren, measureHeight]);

  // Cleanup timers
  useEffect(() => () => clearTimeout(timerRef.current), []);

  const layerClass = (base: string): string => {
    if (phase === 'exit') return `${base} exiting`;
    if (phase === 'enter') return `${base} entering`;
    return `${base} visible`;
  };

  // After entering, remove opacity:0 on next frame
  useEffect(() => {
    if (phase === 'enter') {
      const raf = requestAnimationFrame(() => {
        setPhase('enter'); // Force re-render with visible
        const el = contentRef.current;
        if (el) {
          // Force reflow then add visible
          void el.offsetHeight;
          el.classList.remove('entering');
          el.classList.add('visible');
        }
      });
      return () => cancelAnimationFrame(raf);
    }
  }, [phase]);

  const durationVar = { '--duration-smooth': `${duration}ms` } as React.CSSProperties;

  return (
    <div
      ref={containerRef}
      className={`content-switch${className ? ` ${className}` : ''}`}
      style={{ ...durationVar, ...(height !== undefined ? { height } : {}) }}
    >
      {phase === 'skeleton' ? (
        <div className="content-switch-layer visible">
          {skeleton ?? <Skeleton variant="pane" />}
        </div>
      ) : (
        <div ref={contentRef} className={layerClass('content-switch-layer')}>
          {displayedChildren}
        </div>
      )}
    </div>
  );
}
