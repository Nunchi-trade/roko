import { useEffect, useRef } from 'react';
import * as THREE from 'three';

/** Tier color distribution: T0 78%, T1 17%, T2 5% */
export const TIER_COLORS = {
  T0: 0x7a5060,  // rose-dim
  T1: 0xcc90a8,  // rose-bright
  T2: 0xc8b890,  // bone
} as const;

export function pickTierColor(roll: number): number {
  if (roll < 0.78) return TIER_COLORS.T0;
  if (roll < 0.95) return TIER_COLORS.T1;
  return TIER_COLORS.T2;
}

export interface ThreeSceneOptions {
  antialias?: boolean;
  alpha?: boolean;
  clearColor?: number;
  clearAlpha?: number;
  cameraFov?: number;
  cameraNear?: number;
  cameraFar?: number;
  cameraZ?: number;
}

export interface ThreeSceneResult {
  scene: THREE.Scene;
  camera: THREE.PerspectiveCamera;
  renderer: THREE.WebGLRenderer;
  cleanup: () => void;
}

const DEFAULTS: Required<ThreeSceneOptions> = {
  antialias: true,
  alpha: true,
  clearColor: 0x000000,
  clearAlpha: 0,
  cameraFov: 60,
  cameraNear: 0.1,
  cameraFar: 100,
  cameraZ: 5,
};

export function useThreeScene(
  containerRef: React.RefObject<HTMLDivElement | null>,
  options?: ThreeSceneOptions,
): ThreeSceneResult | null {
  const resultRef = useRef<ThreeSceneResult | null>(null);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const opts = { ...DEFAULTS, ...options };

    const renderer = new THREE.WebGLRenderer({
      antialias: opts.antialias,
      alpha: opts.alpha,
    });
    renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
    renderer.setClearColor(opts.clearColor, opts.clearAlpha);

    const rect = el.getBoundingClientRect();
    renderer.setSize(rect.width, rect.height);
    el.appendChild(renderer.domElement);

    const camera = new THREE.PerspectiveCamera(
      opts.cameraFov,
      rect.width / rect.height,
      opts.cameraNear,
      opts.cameraFar,
    );
    camera.position.z = opts.cameraZ;

    const scene = new THREE.Scene();

    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const { width, height } = entry.contentRect;
        if (width === 0 || height === 0) continue;
        renderer.setSize(width, height);
        camera.aspect = width / height;
        camera.updateProjectionMatrix();
      }
    });
    ro.observe(el);

    const cleanup = () => {
      ro.disconnect();
      renderer.dispose();
      if (renderer.domElement.parentNode) {
        renderer.domElement.parentNode.removeChild(renderer.domElement);
      }
    };

    resultRef.current = { scene, camera, renderer, cleanup };

    return cleanup;
  }, [containerRef, options]);

  return resultRef.current;
}
