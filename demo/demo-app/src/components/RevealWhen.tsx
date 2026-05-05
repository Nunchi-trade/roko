import type { ReactNode, CSSProperties } from 'react';
import './RevealWhen.css';

export type RevealMode = 'fade' | 'slide-up' | 'scale' | 'blur' | 'clip';

interface RevealWhenProps {
  visible: boolean;
  children: ReactNode;
  /** Animation mode (default: "fade") */
  mode?: RevealMode;
  /** Animation duration in ms (default: 350) */
  duration?: number;
  /** Animation delay in ms (default: 0) */
  delay?: number;
  /** Additional className */
  className?: string;
}

export default function RevealWhen({
  visible,
  children,
  mode = 'fade',
  duration = 350,
  delay = 0,
  className,
}: RevealWhenProps) {
  if (!visible) return null;

  const style: CSSProperties = {
    animationDuration: `${duration}ms`,
    animationDelay: delay > 0 ? `${delay}ms` : undefined,
  };

  return (
    <div
      className={`reveal-when reveal-when--${mode}${className ? ` ${className}` : ''}`}
      style={style}
    >
      {children}
    </div>
  );
}
