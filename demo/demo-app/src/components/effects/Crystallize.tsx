import { useEffect, useRef, useState, type ReactNode } from 'react';
import './Crystallize.css';

interface CrystallizeProps {
  trigger: boolean;
  color?: string;       // default 'var(--rose-bright)'
  particleCount?: number; // default 10
  children: ReactNode;
}

interface Particle {
  id: number;
  angle: number;   // radians
  distance: number; // px
  size: number;     // px
  delay: number;    // ms
}

let particleIdCounter = 0;

export function Crystallize({
  trigger,
  color = 'var(--rose-bright)',
  particleCount = 10,
  children,
}: CrystallizeProps) {
  const prevTrigger = useRef(trigger);
  const [particles, setParticles] = useState<Particle[]>([]);
  const [shimmer, setShimmer] = useState(false);

  useEffect(() => {
    // Only fire on false -> true transition
    if (trigger && !prevTrigger.current) {
      // Spawn particles
      const newParticles: Particle[] = [];
      for (let i = 0; i < particleCount; i++) {
        newParticles.push({
          id: particleIdCounter++,
          angle: (Math.PI * 2 * i) / particleCount + (Math.random() - 0.5) * 0.6,
          distance: 30 + Math.random() * 40,
          size: 3 + Math.random() * 4,
          delay: Math.random() * 100,
        });
      }
      setParticles(newParticles);
      setShimmer(true);

      // Clean up particles after animation
      const particleTimer = setTimeout(() => setParticles([]), 900);
      const shimmerTimer = setTimeout(() => setShimmer(false), 250);

      prevTrigger.current = trigger;
      return () => {
        clearTimeout(particleTimer);
        clearTimeout(shimmerTimer);
      };
    }
    prevTrigger.current = trigger;
  }, [trigger, particleCount]);

  return (
    <div className="crystallize">
      {children}

      {shimmer && (
        <div
          className="crystallize__shimmer"
          style={{ background: color }}
        />
      )}

      {particles.map((p) => (
        <span
          key={p.id}
          className="crystallize__particle"
          style={{
            '--cx-angle': `${Math.cos(p.angle) * p.distance}px`,
            '--cy-angle': `${Math.sin(p.angle) * p.distance}px`,
            '--cx-size': `${p.size}px`,
            width: p.size,
            height: p.size,
            background: color,
            animationDelay: `${p.delay}ms`,
          } as React.CSSProperties}
        />
      ))}
    </div>
  );
}
