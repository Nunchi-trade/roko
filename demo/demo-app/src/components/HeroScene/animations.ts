import * as THREE from 'three';
import { COL, DUST_COUNT, type SceneObjects } from './scene-setup';

const RING_R = 5.0;

export interface AnimationState {
  time: number;
  raf: number;
  mouse: { x: number; y: number };
}

/** Set up mouse-tracking listeners. Returns the mouse state object and a cleanup function. */
export function initMouseTracking(): { mouse: { x: number; y: number }; cleanup: () => void } {
  const mouse = { x: 0, y: 0 };

  function onMouseMove(e: MouseEvent) {
    mouse.x = (e.clientX / window.innerWidth) * 2 - 1;
    mouse.y = -(e.clientY / window.innerHeight) * 2 + 1;
  }

  window.addEventListener('mousemove', onMouseMove);

  return {
    mouse,
    cleanup: () => window.removeEventListener('mousemove', onMouseMove),
  };
}

/** Create the animation tick function that drives the scene each frame */
export function createAnimationLoop(
  objs: SceneObjects,
  camera: THREE.PerspectiveCamera,
  renderer: THREE.WebGLRenderer,
  scene: THREE.Scene,
  activeStepRef: { current: number },
  mouse: { x: number; y: number },
): AnimationState {
  const state: AnimationState = { time: 0, raf: 0, mouse };
  const baseCam = { x: 0, y: 6, z: 16 };

  function tick() {
    state.time += 0.005;
    const time = state.time;

    // Primary ring -- very slow rotation
    objs.primaryGroup.rotation.y = time * 0.12;

    // Secondary + tertiary -- different speeds
    objs.secondaryGroup.rotation.y = -time * 0.08;
    objs.tertiaryGroup.rotation.y = time * 0.05;

    // Decorative arcs -- gentle drift
    objs.arcGroup.rotation.y = time * 0.03;
    objs.arcGroup.rotation.x = Math.sin(time * 0.2) * 0.05;

    // Core breathing + rotation
    const breath = 1 + Math.sin(time * 1.0) * 0.12;
    objs.coreL0.scale.setScalar(breath);
    objs.coreL1.scale.setScalar(breath * 0.95);
    objs.coreL2.scale.setScalar(breath * 0.85);
    objs.coreL0.rotation.y = time * 0.3;
    objs.coreL0.rotation.x = time * 0.2;
    objs.coreL1.rotation.y = -time * 0.4;
    objs.coreL1.rotation.z = time * 0.15;
    objs.coreL2.rotation.y = time * 0.5;

    // Core glow pulse
    const corePulse = 0.15 + Math.sin(time * 1.5) * 0.08;
    (objs.coreL2.material as THREE.MeshBasicMaterial).opacity = corePulse;

    // Node highlight based on active step
    const active = activeStepRef.current;
    for (let i = 0; i < 8; i++) {
      const isActive = i === active;
      const wireMat = objs.nodeMeshes[i].material as THREE.MeshBasicMaterial;
      const glowMat = objs.nodeGlowMeshes[i].material as THREE.MeshBasicMaterial;
      const outerMat = objs.nodeOuterMeshes[i].material as THREE.MeshBasicMaterial;

      if (isActive) {
        wireMat.color.setHex(COL.roseGlow);
        wireMat.opacity = 0.9;
        glowMat.opacity = 0.45;
        outerMat.color.setHex(COL.roseDim);
        outerMat.opacity = 0.3;
        const pulse = 1 + Math.sin(time * 3.5) * 0.18;
        objs.nodeMeshes[i].scale.setScalar(pulse);
        objs.nodeGlowMeshes[i].scale.setScalar(pulse);
        objs.nodeOuterMeshes[i].scale.setScalar(pulse * 1.1);
      } else {
        wireMat.color.setHex(COL.rose);
        wireMat.opacity = 0.35;
        glowMat.opacity = 0.08;
        outerMat.color.setHex(COL.dim);
        outerMat.opacity = 0.1;
        objs.nodeMeshes[i].scale.setScalar(1);
        objs.nodeGlowMeshes[i].scale.setScalar(1);
        objs.nodeOuterMeshes[i].scale.setScalar(1);
      }

      // Counter-rotate nodes for visual interest
      objs.nodeMeshes[i].rotation.y = time * 0.4 + i * 0.3;
      objs.nodeMeshes[i].rotation.x = time * 0.25;
      objs.nodeGlowMeshes[i].rotation.y = -time * 0.35;
      objs.nodeOuterMeshes[i].rotation.y = time * 0.15 + i * 0.5;
      objs.nodeOuterMeshes[i].rotation.z = time * 0.1;
    }

    // Orbiting tokens with trails
    for (let i = 0; i < 3; i++) {
      const angle = time * objs.tokenSpeeds[i] + objs.tokenOffsets[i];
      const bobY = Math.sin(time * 1.2 + objs.tokenOffsets[i]) * 0.25;
      const x = Math.cos(angle) * RING_R;
      const z = Math.sin(angle) * RING_R;
      objs.tokens[i].position.set(x, bobY, z);

      // Trail positions (offset backward in time)
      for (let t = 0; t < 4; t++) {
        const trailAngle = angle - (t + 1) * 0.08;
        const trailBob = Math.sin(time * 1.2 + objs.tokenOffsets[i] - (t + 1) * 0.05) * 0.25;
        objs.tokenTrails[i][t].position.set(
          Math.cos(trailAngle) * RING_R,
          trailBob,
          Math.sin(trailAngle) * RING_R,
        );
      }
    }

    // Dust drift
    const posArr = objs.dustGeo.attributes.position.array as Float32Array;
    for (let i = 0; i < DUST_COUNT; i++) {
      const ix = i * 3;
      posArr[ix] += objs.dustVelocities[ix];
      posArr[ix + 1] += objs.dustVelocities[ix + 1] + Math.sin(time + objs.dustPhases[i]) * 0.001;
      posArr[ix + 2] += objs.dustVelocities[ix + 2];

      // Soft wrap
      if (posArr[ix] > 10) posArr[ix] = -10;
      if (posArr[ix] < -10) posArr[ix] = 10;
      if (posArr[ix + 1] > 4) posArr[ix + 1] = -4;
      if (posArr[ix + 1] < -4) posArr[ix + 1] = 4;
      if (posArr[ix + 2] > 10) posArr[ix + 2] = -10;
      if (posArr[ix + 2] < -10) posArr[ix + 2] = 10;
    }
    objs.dustGeo.attributes.position.needsUpdate = true;

    // Mouse parallax
    camera.position.x = baseCam.x + state.mouse.x * 1.5;
    camera.position.y = baseCam.y + state.mouse.y * 1.0;
    camera.lookAt(0, 0, 0);

    renderer.render(scene, camera);
    state.raf = requestAnimationFrame(tick);
  }

  tick();
  return state;
}
