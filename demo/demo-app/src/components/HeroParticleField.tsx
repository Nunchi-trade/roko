import AmbientParticles, { type ParticleFieldConfig } from './AmbientParticles';

/**
 * Hero-specific particle field config.
 * Uses the default rosedust rose + bone palette at standard density.
 * Consumers wanting custom particles should use AmbientParticles directly.
 */
const HERO_CONFIG: ParticleFieldConfig = {
  count: 30,
  speed: 0.00012,
  colors: [[220, 165, 189], [200, 184, 144]],
  minSize: 0.3,
  maxSize: 1.7,
  baseAlpha: 0.18,
  alphaSwing: 0.12,
  glowRadius: 8,
  animSpeed: 1.0,
  reactivity: 0,
};

export default function HeroParticleField() {
  return <AmbientParticles config={HERO_CONFIG} />;
}
