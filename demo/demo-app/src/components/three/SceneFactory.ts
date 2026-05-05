import { useEffect, useState, type RefObject } from 'react';
import * as THREE from 'three';

export { TIER_COLORS, pickTierColor } from '../../lib/three-shared';

/* ── cached geometry ── */
let _octaGeo: THREE.OctahedronGeometry | null = null;

export function createOctahedronGeometry(radius = 0.06): THREE.OctahedronGeometry {
  if (!_octaGeo || _octaGeo.parameters.radius !== radius) {
    _octaGeo = new THREE.OctahedronGeometry(radius, 0);
  }
  return _octaGeo;
}

/* ── rose-dust material factory ── */
export function createRoseDustMaterial(opts?: {
  color?: number;
  opacity?: number;
  wireframe?: boolean;
}): THREE.MeshStandardMaterial {
  const { color = 0x7a5060, opacity = 0.7, wireframe = true } = opts ?? {};
  return new THREE.MeshStandardMaterial({
    color,
    transparent: true,
    opacity,
    wireframe,
  });
}

/* ── IntersectionObserver visibility gate ── */
export function useIntersectionGate(
  containerRef: RefObject<HTMLElement | null>,
): boolean {
  const [isVisible, setIsVisible] = useState(false);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const observer = new IntersectionObserver(
      ([entry]) => setIsVisible(entry.isIntersecting),
      { threshold: 0.05 },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [containerRef]);

  return isVisible;
}
