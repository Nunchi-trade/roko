/**
 * Three.js scene lifecycle: creation, disposal, and utility helpers.
 */
import * as THREE from 'three';

export interface SceneKit {
  renderer: THREE.WebGLRenderer;
  camera: THREE.PerspectiveCamera;
  scene: THREE.Scene;
  clock: THREE.Clock;
  dispose: () => void;
  resize: (w: number, h: number) => void;
}

export interface SceneKitOpts {
  fov?: number;
  near?: number;
  far?: number;
  fog?: { color: number; density: number };
  clearColor?: number;
  clearAlpha?: number;
}

/**
 * Create renderer + camera + scene inside the given container.
 */
export function createSceneKit(container: HTMLElement, opts: SceneKitOpts = {}): SceneKit {
  const { fov = 45, near = 0.1, far = 200, clearColor = 0x060608, clearAlpha = 0 } = opts;

  const rect = container.getBoundingClientRect();
  const scene = new THREE.Scene();
  if (opts.fog) {
    scene.fog = new THREE.FogExp2(opts.fog.color, opts.fog.density);
  }

  const camera = new THREE.PerspectiveCamera(fov, rect.width / rect.height, near, far);

  const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.setClearColor(clearColor, clearAlpha);
  renderer.setSize(rect.width, rect.height);
  container.appendChild(renderer.domElement);

  const clock = new THREE.Clock();

  function resize(w: number, h: number) {
    if (w === 0 || h === 0) return;
    renderer.setSize(w, h);
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
  }

  function dispose() {
    disposeScene(scene, renderer);
  }

  return { renderer, camera, scene, clock, dispose, resize };
}

/**
 * Full traversal cleanup — disposes all geometries, materials, and textures.
 */
export function disposeScene(scene: THREE.Scene, renderer: THREE.WebGLRenderer): void {
  scene.traverse((obj) => {
    if (obj instanceof THREE.Mesh || obj instanceof THREE.Points) {
      obj.geometry.dispose();
      if (Array.isArray(obj.material)) {
        obj.material.forEach((m) => m.dispose());
      } else {
        obj.material.dispose();
      }
    }
    if (obj instanceof THREE.Line || obj instanceof THREE.LineSegments) {
      obj.geometry.dispose();
      (obj.material as THREE.Material).dispose();
    }
  });
  renderer.dispose();
  if (renderer.domElement.parentNode) {
    renderer.domElement.parentNode.removeChild(renderer.domElement);
  }
}

/**
 * WebGL capability check (matches HeroScene pattern).
 */
export function canRunWebGL(): boolean {
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
 * Exponential interpolation — smooths camera and value transitions.
 * `factor` is the approach rate per second (0.03–0.06 typical per-frame).
 */
export function expLerp(current: number, target: number, factor: number): number {
  return current + (target - current) * factor;
}

/**
 * Normalized mouse tracker for parallax effects.
 * Returns `{ mx, my }` in [-1, 1] range with exponential smoothing.
 */
export function createMouseTracker(): {
  mouse: { x: number; y: number };
  target: { x: number; y: number };
  update: (factor?: number) => void;
  cleanup: () => void;
} {
  const mouse = { x: 0, y: 0 };
  const target = { x: 0, y: 0 };

  function onPointerMove(e: PointerEvent) {
    target.x = (e.clientX / window.innerWidth - 0.5) * 2;
    target.y = (e.clientY / window.innerHeight - 0.5) * 2;
  }

  window.addEventListener('pointermove', onPointerMove);

  function update(factor = 0.06) {
    mouse.x += (target.x - mouse.x) * factor;
    mouse.y += (target.y - mouse.y) * factor;
  }

  function cleanup() {
    window.removeEventListener('pointermove', onPointerMove);
  }

  return { mouse, target, update, cleanup };
}
