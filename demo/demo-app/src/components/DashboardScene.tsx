/**
 * Generic Three.js dashboard wrapper.
 *
 * Creates a full-viewport WebGL scene, runs a RAF loop, handles resize,
 * and falls back to AmbientParticles on no-WebGL. Children are rendered
 * as absolute-positioned overlays on top of the canvas.
 */
import { useEffect, useRef, type ReactNode } from 'react';
import * as THREE from 'three';
import { createSceneKit, canRunWebGL, createMouseTracker, type SceneKit } from '../lib/three/scene-lifecycle';
import { makeGlowSprite } from '../lib/three/primitives';
import AmbientParticles from './AmbientParticles';

export interface DashboardSceneProps {
  /** Build scene objects. Called once on mount. */
  setup: (scene: THREE.Scene, sprite: THREE.Texture, kit: SceneKit) => void;
  /** Per-frame animation. */
  animate: (dt: number, time: number, mouse: { x: number; y: number }, kit: SceneKit) => void;
  /** Initial camera position [x, y, z]. */
  cameraPos?: [number, number, number];
  /** Optional FogExp2 config. */
  fog?: { color: number; density: number };
  /** Overlay children (GlassPanels, drawers). */
  children?: ReactNode;
  className?: string;
}

export default function DashboardScene({
  setup,
  animate,
  cameraPos = [0, 8, 18],
  fog,
  children,
  className,
}: DashboardSceneProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const webgl = useRef(canRunWebGL());

  // Keep latest callbacks in refs so RAF closure stays current
  const setupRef = useRef(setup);
  setupRef.current = setup;
  const animateRef = useRef(animate);
  animateRef.current = animate;

  useEffect(() => {
    if (!webgl.current || !containerRef.current) return;

    const el = containerRef.current;
    const kit = createSceneKit(el, { fog });
    kit.camera.position.set(...cameraPos);
    kit.camera.lookAt(0, 0, 0);

    const sprite = makeGlowSprite();
    setupRef.current(kit.scene, sprite, kit);

    const tracker = createMouseTracker();
    let raf = 0;
    let elapsed = 0;

    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const { width, height } = entry.contentRect;
        kit.resize(width, height);
      }
    });
    ro.observe(el);

    function tick() {
      const dt = kit.clock.getDelta();
      elapsed += dt;
      tracker.update();
      animateRef.current(dt, elapsed, tracker.mouse, kit);
      kit.renderer.render(kit.scene, kit.camera);
      raf = requestAnimationFrame(tick);
    }
    raf = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      tracker.cleanup();
      sprite.dispose();
      kit.dispose();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!webgl.current) {
    return (
      <div className={`dashboard-scene ${className ?? ''}`} style={{ position: 'relative', width: '100%', height: '100%' }}>
        <AmbientParticles />
        <div style={{ position: 'absolute', inset: 0, pointerEvents: 'auto' }}>
          {children}
        </div>
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      className={`dashboard-scene ${className ?? ''}`}
      style={{ position: 'relative', width: '100%', height: '100%', overflow: 'hidden' }}
    >
      {/* canvas is appended by createSceneKit */}
      <div style={{ position: 'absolute', inset: 0, pointerEvents: 'none', zIndex: 10 }}>
        <div style={{ pointerEvents: 'auto', width: '100%', height: '100%', position: 'relative' }}>
          {children}
        </div>
      </div>
    </div>
  );
}
