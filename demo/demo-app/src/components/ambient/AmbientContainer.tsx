import type { ReactNode } from 'react';
import NoiseBackground from './NoiseBackground';
import FluidGradient from './FluidGradient';
import './AmbientContainer.css';

interface AmbientContainerProps {
  effect?: 'noise' | 'fluid' | 'none';
  intensity?: number;
  children: ReactNode;
  className?: string;
}

export default function AmbientContainer({
  effect = 'noise',
  intensity = 0.5,
  children,
  className,
}: AmbientContainerProps) {
  const clamped = Math.max(0, Math.min(1, intensity));

  return (
    <div className={`ambient-container ${className ?? ''}`}>
      <div className="ambient-container__bg">
        {effect === 'noise' && (
          <NoiseBackground
            density={Math.round(2 + (1 - clamped) * 6)}
            opacity={0.05 + clamped * 0.2}
          />
        )}
        {effect === 'fluid' && (
          <FluidGradient opacity={0.1 + clamped * 0.4} />
        )}
      </div>
      <div className="ambient-container__content">{children}</div>
    </div>
  );
}
