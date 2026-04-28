import { useEffect, useRef, useState, useCallback } from 'react';
import * as THREE from 'three';
import { pickTierColor } from '../lib/three-shared';
import AmbientParticles from './AmbientParticles';

const COUNT = 600;
const TAU = Math.PI * 2;

function canRunThreeJS(): boolean {
  try {
    if (typeof navigator !== 'undefined' && navigator.hardwareConcurrency < 4) return false;
    const c = document.createElement('canvas');
    const gl = c.getContext('webgl2') || c.getContext('webgl');
    return gl !== null;
  } catch {
    return false;
  }
}

/**
 * Subtle ambient particle dust — tiny glowing points that drift slowly.
 * NOT the previous giant octahedron rocks. These are meant to be atmospheric,
 * barely-visible, like dust motes in a dark room.
 */
function ThreeParticleField() {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const renderer = new THREE.WebGLRenderer({ antialias: false, alpha: true });
    renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
    renderer.setClearColor(0x000000, 0);
    const rect = el.getBoundingClientRect();
    renderer.setSize(rect.width, rect.height);
    el.appendChild(renderer.domElement);

    const camera = new THREE.PerspectiveCamera(60, rect.width / rect.height, 0.1, 200);
    camera.position.z = 30;

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

    // Create a small glowing sprite texture
    const spriteCanvas = document.createElement('canvas');
    spriteCanvas.width = 32;
    spriteCanvas.height = 32;
    const sctx = spriteCanvas.getContext('2d')!;
    const gradient = sctx.createRadialGradient(16, 16, 0, 16, 16, 16);
    gradient.addColorStop(0, 'rgba(255,255,255,1)');
    gradient.addColorStop(0.3, 'rgba(255,255,255,0.3)');
    gradient.addColorStop(1, 'rgba(255,255,255,0)');
    sctx.fillStyle = gradient;
    sctx.fillRect(0, 0, 32, 32);
    const spriteTexture = new THREE.CanvasTexture(spriteCanvas);

    // Points geometry — tiny dots, not meshes
    const positions = new Float32Array(COUNT * 3);
    const colors = new Float32Array(COUNT * 3);
    const sizes = new Float32Array(COUNT);
    const phases = new Float32Array(COUNT);
    const velocities = new Float32Array(COUNT * 3);

    for (let i = 0; i < COUNT; i++) {
      // Spread across a wide area
      positions[i * 3] = (Math.random() - 0.5) * 60;
      positions[i * 3 + 1] = (Math.random() - 0.5) * 40;
      positions[i * 3 + 2] = (Math.random() - 0.5) * 20;

      const roll = Math.random();
      const c = new THREE.Color(pickTierColor(roll));
      colors[i * 3] = c.r;
      colors[i * 3 + 1] = c.g;
      colors[i * 3 + 2] = c.b;

      sizes[i] = 0.15 + Math.random() * 0.35;
      phases[i] = Math.random() * TAU;

      velocities[i * 3] = (Math.random() - 0.5) * 0.008;
      velocities[i * 3 + 1] = (Math.random() - 0.5) * 0.006;
      velocities[i * 3 + 2] = (Math.random() - 0.5) * 0.003;
    }

    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
    geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
    geo.setAttribute('size', new THREE.BufferAttribute(sizes, 1));

    const mat = new THREE.PointsMaterial({
      map: spriteTexture,
      size: 0.4,
      sizeAttenuation: true,
      transparent: true,
      opacity: 0.35,
      vertexColors: true,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
    });

    const points = new THREE.Points(geo, mat);
    scene.add(points);

    // Mouse interaction
    const mouse = { x: 9999, y: 9999 };
    function onMouseMove(e: MouseEvent) {
      mouse.x = (e.clientX / window.innerWidth) * 2 - 1;
      mouse.y = -(e.clientY / window.innerHeight) * 2 + 1;
    }
    window.addEventListener('mousemove', onMouseMove);

    let time = 0;
    let raf: number;

    function tick() {
      time += 0.004;

      const posArr = geo.attributes.position.array as Float32Array;

      for (let i = 0; i < COUNT; i++) {
        const ix = i * 3;
        const iy = ix + 1;
        const iz = ix + 2;

        // Gentle drift
        posArr[ix] += velocities[ix];
        posArr[iy] += velocities[iy];
        posArr[iz] += velocities[iz];

        // Subtle breathing motion
        const breath = Math.sin(time * 1.5 + phases[i]) * 0.003;
        posArr[iy] += breath;

        // Soft bounds — wrap around
        if (posArr[ix] > 30) posArr[ix] = -30;
        if (posArr[ix] < -30) posArr[ix] = 30;
        if (posArr[iy] > 20) posArr[iy] = -20;
        if (posArr[iy] < -20) posArr[iy] = 20;
        if (posArr[iz] > 10) posArr[iz] = -10;
        if (posArr[iz] < -10) posArr[iz] = 10;

        // Mouse repulsion (very gentle)
        const screenX = mouse.x * 30;
        const screenY = mouse.y * 20;
        const dx = posArr[ix] - screenX;
        const dy = posArr[iy] - screenY;
        const distSq = dx * dx + dy * dy;
        if (distSq < 25 && distSq > 0.1) {
          const force = 0.01 / distSq;
          velocities[ix] += dx * force;
          velocities[iy] += dy * force;
        }

        // Damping
        velocities[ix] *= 0.999;
        velocities[iy] *= 0.999;
        velocities[iz] *= 0.999;
      }

      geo.attributes.position.needsUpdate = true;

      // Slow camera sway
      camera.position.x = Math.sin(time * 0.3) * 1.5;
      camera.position.y = Math.cos(time * 0.2) * 1;
      camera.lookAt(0, 0, 0);

      renderer.render(scene, camera);
      raf = requestAnimationFrame(tick);
    }

    tick();

    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener('mousemove', onMouseMove);
      ro.disconnect();
      geo.dispose();
      mat.dispose();
      spriteTexture.dispose();
      renderer.dispose();
      if (renderer.domElement.parentNode) {
        renderer.domElement.parentNode.removeChild(renderer.domElement);
      }
    };
  }, []);

  return (
    <div
      ref={containerRef}
      style={{ position: 'fixed', inset: 0, zIndex: 0, pointerEvents: 'none' }}
    />
  );
}

export default function HeroParticleField() {
  const [useThree, setUseThree] = useState<boolean | null>(null);

  const checkCapability = useCallback(() => {
    setUseThree(canRunThreeJS());
  }, []);

  useEffect(() => {
    checkCapability();
  }, [checkCapability]);

  if (useThree === null) return null;
  if (!useThree) return <AmbientParticles />;
  return <ThreeParticleField />;
}
