/**
 * ParticleField — ambient barrel re-export of AmbientParticles
 * with configurable density, colors, speed, reactivity, and size.
 *
 * Usage:
 *   import { ParticleField } from '../ambient';
 *   <ParticleField config={{ count: 60, speed: 0.0003, reactivity: 0.5 }} />
 */
export { default as ParticleField } from '../AmbientParticles';
export type { ParticleFieldConfig, AmbientParticlesProps } from '../AmbientParticles';
