import type { ReactNode, CSSProperties } from 'react';

type TransitionType = 'fadeUp' | 'scaleIn' | 'slideRight' | 'fadeIn';

interface TransitionProps {
  type?: TransitionType;
  delay?: number;
  duration?: number;
  className?: string;
  children: ReactNode;
}

const animationMap: Record<TransitionType, string> = {
  fadeUp: 'fadeUp',
  scaleIn: 'scaleIn',
  slideRight: 'slideRight',
  fadeIn: 'fadeIn',
};

export function Transition({
  type = 'fadeUp',
  delay,
  duration = 200,
  className,
  children,
}: TransitionProps) {
  const style: CSSProperties = {
    animation: `${animationMap[type]} ${duration}ms var(--ease-expo, cubic-bezier(0.16, 1, 0.3, 1)) forwards`,
    opacity: 0, // initial state; keyframe fills to 1
    ...(delay != null ? { animationDelay: `${delay}ms` } : {}),
  };

  const cls = className ? `transition-wrapper ${className}` : 'transition-wrapper';

  return (
    <div className={cls} style={style}>
      {children}
    </div>
  );
}
