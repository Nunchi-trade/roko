import { useEffect, useRef, useCallback } from 'react';
import * as THREE from 'three';
import { useThreeScene } from '../../lib/three-shared';
import {
  pickTierColor,
  createOctahedronGeometry,
  createRoseDustMaterial,
  useIntersectionGate,
} from './SceneFactory';

const TAU = Math.PI * 2;
const PARTICLE_COUNT = 800;
const TRANSITION_DURATION = 2.0; // seconds
const MAX_TILT = (3 * Math.PI) / 180; // 3 degrees

type SwarmMode = 'chaos' | 'coordination' | 'pheromone';

interface AgentSwarmProps {
  mode?: SwarmMode;
  className?: string;
}

/* ── per-particle state ── */
interface ParticleState {
  // current interpolated position
  x: Float32Array;
  y: Float32Array;
  z: Float32Array;
  // velocity (used in chaos mode)
  vx: Float32Array;
  vy: Float32Array;
  vz: Float32Array;
  // target position for transitions
  tx: Float32Array;
  ty: Float32Array;
  tz: Float32Array;
  // per-particle phase offset
  phase: Float32Array;
  // color per particle
  colors: Uint32Array;
}

function initParticles(): ParticleState {
  const s: ParticleState = {
    x: new Float32Array(PARTICLE_COUNT),
    y: new Float32Array(PARTICLE_COUNT),
    z: new Float32Array(PARTICLE_COUNT),
    vx: new Float32Array(PARTICLE_COUNT),
    vy: new Float32Array(PARTICLE_COUNT),
    vz: new Float32Array(PARTICLE_COUNT),
    tx: new Float32Array(PARTICLE_COUNT),
    ty: new Float32Array(PARTICLE_COUNT),
    tz: new Float32Array(PARTICLE_COUNT),
    phase: new Float32Array(PARTICLE_COUNT),
    colors: new Uint32Array(PARTICLE_COUNT),
  };

  for (let i = 0; i < PARTICLE_COUNT; i++) {
    // spread in a sphere of radius ~4
    const theta = Math.random() * TAU;
    const phi = Math.acos(2 * Math.random() - 1);
    const r = 1.0 + Math.random() * 3.0;
    s.x[i] = Math.sin(phi) * Math.cos(theta) * r;
    s.y[i] = Math.sin(phi) * Math.sin(theta) * r;
    s.z[i] = Math.cos(phi) * r;

    s.vx[i] = (Math.random() - 0.5) * 0.02;
    s.vy[i] = (Math.random() - 0.5) * 0.02;
    s.vz[i] = (Math.random() - 0.5) * 0.02;

    s.tx[i] = s.x[i];
    s.ty[i] = s.y[i];
    s.tz[i] = s.z[i];

    s.phase[i] = Math.random() * TAU;
    s.colors[i] = pickTierColor(Math.random());
  }

  return s;
}

/* ── target position generators per mode ── */

function computeTargets(mode: SwarmMode, state: ParticleState, _time: number) {
  for (let i = 0; i < PARTICLE_COUNT; i++) {
    const p = state.phase[i];
    switch (mode) {
      case 'chaos': {
        // targets follow brownian drift - no fixed target
        state.tx[i] = state.x[i] + state.vx[i];
        state.ty[i] = state.y[i] + state.vy[i];
        state.tz[i] = state.z[i] + state.vz[i];
        break;
      }
      case 'coordination': {
        // spiral vortex - particles converge into helix
        const t = (i / PARTICLE_COUNT) * TAU * 4 + p * 0.5;
        const spiralR = 0.5 + (i / PARTICLE_COUNT) * 2.5;
        state.tx[i] = Math.cos(t) * spiralR;
        state.ty[i] = ((i / PARTICLE_COUNT) - 0.5) * 5;
        state.tz[i] = Math.sin(t) * spiralR;
        break;
      }
      case 'pheromone': {
        // concentric rings orbiting center
        const ringIdx = i % 5;
        const ringR = 1.0 + ringIdx * 0.7;
        const posInRing = (i / PARTICLE_COUNT) * TAU + p;
        state.tx[i] = Math.cos(posInRing) * ringR;
        state.ty[i] = (ringIdx - 2) * 0.4;
        state.tz[i] = Math.sin(posInRing) * ringR;
        break;
      }
    }
  }
}

export default function AgentSwarm({ mode = 'chaos', className }: AgentSwarmProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const isVisible = useIntersectionGate(containerRef);
  const threeResult = useThreeScene(containerRef, {
    cameraFov: 60,
    cameraZ: 6,
    clearAlpha: 0,
    alpha: true,
  });
  const modeRef = useRef<SwarmMode>(mode);
  const prevModeRef = useRef<SwarmMode>(mode);
  const transitionStartRef = useRef(0);

  // Track mode changes
  useEffect(() => {
    if (mode !== modeRef.current) {
      prevModeRef.current = modeRef.current;
      modeRef.current = mode;
      transitionStartRef.current = performance.now() / 1000;
    }
  }, [mode]);

  // mouse tracking
  const mouseRef = useRef({ x: 0, y: 0 });
  const onMouseMove = useCallback((e: MouseEvent) => {
    mouseRef.current.x = (e.clientX / window.innerWidth) * 2 - 1;
    mouseRef.current.y = (e.clientY / window.innerHeight) * 2 - 1;
  }, []);

  useEffect(() => {
    window.addEventListener('mousemove', onMouseMove);
    return () => window.removeEventListener('mousemove', onMouseMove);
  }, [onMouseMove]);

  // main animation loop
  useEffect(() => {
    if (!threeResult) return;
    const { scene, camera, renderer } = threeResult;

    const state = initParticles();

    // instanced mesh
    const geo = createOctahedronGeometry(0.06);
    const mat = createRoseDustMaterial({ opacity: 0.7, wireframe: true });
    const mesh = new THREE.InstancedMesh(geo, mat, PARTICLE_COUNT);
    mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);

    // per-instance color
    const colorAttr = new Float32Array(PARTICLE_COUNT * 3);
    const tmpColor = new THREE.Color();
    for (let i = 0; i < PARTICLE_COUNT; i++) {
      tmpColor.set(state.colors[i]);
      colorAttr[i * 3] = tmpColor.r;
      colorAttr[i * 3 + 1] = tmpColor.g;
      colorAttr[i * 3 + 2] = tmpColor.b;
    }
    mesh.instanceColor = new THREE.InstancedBufferAttribute(colorAttr, 3);

    scene.add(mesh);

    // lighting
    const hemi = new THREE.HemisphereLight(0xdca5bd, 0x2a1e28, 0.6);
    scene.add(hemi);
    const point = new THREE.PointLight(0xcc90a8, 0.8, 20);
    point.position.set(0, 3, 4);
    scene.add(point);

    const dummy = new THREE.Object3D();
    let raf = 0;
    const baseCamPos = camera.position.clone();

    function animate() {
      raf = requestAnimationFrame(animate);
      const now = performance.now() / 1000;
      const currentMode = modeRef.current;

      // transition blending
      const elapsed = now - transitionStartRef.current;
      const tFactor = Math.min(elapsed / TRANSITION_DURATION, 1);

      computeTargets(currentMode, state, now);

      for (let i = 0; i < PARTICLE_COUNT; i++) {
        if (currentMode === 'chaos') {
          // brownian motion: update velocity with random walk
          state.vx[i] += (Math.random() - 0.5) * 0.004;
          state.vy[i] += (Math.random() - 0.5) * 0.004;
          state.vz[i] += (Math.random() - 0.5) * 0.004;
          // damping
          state.vx[i] *= 0.99;
          state.vy[i] *= 0.99;
          state.vz[i] *= 0.99;
          // apply
          state.x[i] += state.vx[i];
          state.y[i] += state.vy[i];
          state.z[i] += state.vz[i];
          // soft boundary (sphere of radius 5)
          const d = Math.sqrt(state.x[i] ** 2 + state.y[i] ** 2 + state.z[i] ** 2);
          if (d > 5) {
            const scale = 5 / d;
            state.x[i] *= scale;
            state.y[i] *= scale;
            state.z[i] *= scale;
          }
        } else {
          // lerp towards target positions
          const lerpSpeed = 0.03 + tFactor * 0.04;
          state.x[i] += (state.tx[i] - state.x[i]) * lerpSpeed;
          state.y[i] += (state.ty[i] - state.y[i]) * lerpSpeed;
          state.z[i] += (state.tz[i] - state.z[i]) * lerpSpeed;

          // add subtle oscillation in formed modes
          const osc = Math.sin(now * 0.8 + state.phase[i]) * 0.02;
          state.x[i] += osc;
          state.y[i] += Math.cos(now * 0.6 + state.phase[i]) * 0.015;
        }

        // rotation based on phase + time
        const rot = now * 0.5 + state.phase[i];
        dummy.position.set(state.x[i], state.y[i], state.z[i]);
        dummy.rotation.set(rot, rot * 0.7, 0);
        dummy.scale.setScalar(0.8 + Math.sin(now + state.phase[i]) * 0.2);
        dummy.updateMatrix();
        mesh.setMatrixAt(i, dummy.matrix);
      }
      mesh.instanceMatrix.needsUpdate = true;

      // mouse parallax (subtle camera tilt, max 3 degrees)
      const mx = mouseRef.current.x;
      const my = mouseRef.current.y;
      camera.position.x = baseCamPos.x + mx * 0.3;
      camera.position.y = baseCamPos.y - my * 0.3;
      camera.rotation.y = -mx * MAX_TILT;
      camera.rotation.x = my * MAX_TILT;

      renderer.render(scene, camera);
    }

    animate();

    return () => {
      cancelAnimationFrame(raf);
      scene.remove(mesh);
      scene.remove(hemi);
      scene.remove(point);
      mat.dispose();
      mesh.dispose();
      hemi.dispose();
      point.dispose();
    };
  }, [threeResult, isVisible]);

  return (
    <div
      ref={containerRef}
      className={className}
      style={{
        position: 'absolute',
        inset: 0,
        pointerEvents: 'none',
        zIndex: 0,
      }}
    />
  );
}
