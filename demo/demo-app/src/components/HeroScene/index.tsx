import { useEffect, useRef, useState, useCallback } from 'react';
import * as THREE from 'three';
import AmbientParticles from '../AmbientParticles';
import { canRunWebGL, makeGlowSprite, buildScene } from './scene-setup';
import { initMouseTracking, createAnimationLoop } from './animations';

interface HeroSceneCanvasProps {
  activeStep: number;
}

function HeroSceneCanvas({ activeStep }: HeroSceneCanvasProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const activeStepRef = useRef(activeStep);
  activeStepRef.current = activeStep;

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
    renderer.setClearColor(0x000000, 0);
    const rect = el.getBoundingClientRect();
    renderer.setSize(rect.width, rect.height);
    el.appendChild(renderer.domElement);

    const camera = new THREE.PerspectiveCamera(50, rect.width / rect.height, 0.1, 200);
    camera.position.set(0, 6, 16);
    camera.lookAt(0, 0, 0);

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

    const sprite = makeGlowSprite();
    const sceneObjs = buildScene(scene, sprite);
    const { mouse, cleanup: cleanupMouse } = initMouseTracking();
    const animState = createAnimationLoop(sceneObjs, camera, renderer, scene, activeStepRef, mouse);

    return () => {
      cancelAnimationFrame(animState.raf);
      cleanupMouse();
      ro.disconnect();
      scene.traverse((obj) => {
        if (obj instanceof THREE.Mesh || obj instanceof THREE.Points) {
          obj.geometry.dispose();
          if (Array.isArray(obj.material)) obj.material.forEach((m) => m.dispose());
          else obj.material.dispose();
        }
        if (obj instanceof THREE.Line || obj instanceof THREE.LineSegments) {
          obj.geometry.dispose();
          (obj.material as THREE.Material).dispose();
        }
      });
      sprite.dispose();
      renderer.dispose();
      if (renderer.domElement.parentNode) {
        renderer.domElement.parentNode.removeChild(renderer.domElement);
      }
    };
  }, []);

  return <div ref={containerRef} className="hero-scene-canvas" />;
}

/* ── exported with capability detection ── */
interface HeroSceneProps {
  activeStep: number;
}

export default function HeroScene({ activeStep }: HeroSceneProps) {
  const [useThree, setUseThree] = useState<boolean | null>(null);
  const check = useCallback(() => setUseThree(canRunWebGL()), []);
  useEffect(() => { check(); }, [check]);

  if (useThree === null) return null;
  if (!useThree) return <AmbientParticles />;
  return <HeroSceneCanvas activeStep={activeStep} />;
}
