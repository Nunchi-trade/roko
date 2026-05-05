/**
 * Reusable Three.js geometry builders.
 *
 * Extracted from HeroScene/scene-setup.ts + new builders for dashboard scenes.
 */
import * as THREE from 'three';
import { TAU, COL } from './constants';

/* ── Existing primitives (from HeroScene) ───────────────────── */

export function makeGlowSprite(): THREE.Texture {
  const c = document.createElement('canvas');
  c.width = 64;
  c.height = 64;
  const ctx = c.getContext('2d')!;
  const g = ctx.createRadialGradient(32, 32, 0, 32, 32, 32);
  g.addColorStop(0, 'rgba(255,255,255,1)');
  g.addColorStop(0.2, 'rgba(255,255,255,0.5)');
  g.addColorStop(0.5, 'rgba(255,255,255,0.15)');
  g.addColorStop(1, 'rgba(255,255,255,0)');
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, 64, 64);
  return new THREE.CanvasTexture(c);
}

export function makeLineRing(
  radius: number, segments: number, color: number, opacity: number,
): THREE.Line {
  const pts: THREE.Vector3[] = [];
  for (let i = 0; i <= segments; i++) {
    const a = (i / segments) * TAU;
    pts.push(new THREE.Vector3(Math.cos(a) * radius, 0, Math.sin(a) * radius));
  }
  const geo = new THREE.BufferGeometry().setFromPoints(pts);
  const mat = new THREE.LineBasicMaterial({ color, transparent: true, opacity });
  return new THREE.Line(geo, mat);
}

export function makeTickMarks(
  radius: number, count: number, tickLen: number,
  color: number, opacity: number,
): THREE.LineSegments {
  const positions: number[] = [];
  for (let i = 0; i < count; i++) {
    const a = (i / count) * TAU;
    const cos = Math.cos(a);
    const sin = Math.sin(a);
    positions.push(cos * radius, 0, sin * radius);
    positions.push(cos * (radius + tickLen), 0, sin * (radius + tickLen));
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.Float32BufferAttribute(positions, 3));
  const mat = new THREE.LineBasicMaterial({ color, transparent: true, opacity });
  return new THREE.LineSegments(geo, mat);
}

export function makeArc(
  radius: number, startAngle: number, endAngle: number,
  segments: number, color: number, opacity: number,
): THREE.Line {
  const pts: THREE.Vector3[] = [];
  for (let i = 0; i <= segments; i++) {
    const a = startAngle + (endAngle - startAngle) * (i / segments);
    pts.push(new THREE.Vector3(Math.cos(a) * radius, 0, Math.sin(a) * radius));
  }
  const geo = new THREE.BufferGeometry().setFromPoints(pts);
  const mat = new THREE.LineBasicMaterial({ color, transparent: true, opacity });
  return new THREE.Line(geo, mat);
}

/* ── New primitives ─────────────────────────────────────────── */

/**
 * Triple-layered octahedron: wireframe + solid core + outer halo.
 * Returns a Group so all three share the same position/rotation.
 */
export function makeTripleNode(
  size: number,
  wireColor: number,
  glowColor: number,
): {
  group: THREE.Group;
  wire: THREE.Mesh;
  solid: THREE.Mesh;
  halo: THREE.Mesh;
} {
  const group = new THREE.Group();

  const wire = new THREE.Mesh(
    new THREE.OctahedronGeometry(size, 0),
    new THREE.MeshBasicMaterial({
      color: wireColor, wireframe: true, transparent: true, opacity: 0.6,
    }),
  );
  group.add(wire);

  const solid = new THREE.Mesh(
    new THREE.OctahedronGeometry(size * 0.6, 0),
    new THREE.MeshBasicMaterial({
      color: glowColor, transparent: true, opacity: 0.2,
    }),
  );
  group.add(solid);

  const halo = new THREE.Mesh(
    new THREE.OctahedronGeometry(size * 1.4, 0),
    new THREE.MeshBasicMaterial({
      color: wireColor, wireframe: true, transparent: true, opacity: 0.12,
    }),
  );
  group.add(halo);

  return { group, wire, solid, halo };
}

/**
 * Ambient dust particle cloud with vertex colors.
 */
export function makeDustField(
  count: number,
  spread: number,
  sprite: THREE.Texture,
  palette: number[] = [COL.faint, COL.roseDim, COL.rose, COL.bone],
): {
  points: THREE.Points;
  geo: THREE.BufferGeometry;
  phases: Float32Array;
  velocities: Float32Array;
} {
  const positions = new Float32Array(count * 3);
  const colors = new Float32Array(count * 3);
  const phases = new Float32Array(count);
  const velocities = new Float32Array(count * 3);

  for (let i = 0; i < count; i++) {
    const angle = Math.random() * TAU;
    const r = 1.0 + Math.random() * spread;
    const y = (Math.random() - 0.5) * spread * 0.6;
    positions[i * 3] = Math.cos(angle) * r;
    positions[i * 3 + 1] = y;
    positions[i * 3 + 2] = Math.sin(angle) * r;

    const c = new THREE.Color(palette[i % palette.length]);
    colors[i * 3] = c.r;
    colors[i * 3 + 1] = c.g;
    colors[i * 3 + 2] = c.b;

    phases[i] = Math.random() * TAU;
    velocities[i * 3] = (Math.random() - 0.5) * 0.003;
    velocities[i * 3 + 1] = (Math.random() - 0.5) * 0.002;
    velocities[i * 3 + 2] = (Math.random() - 0.5) * 0.003;
  }

  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  geo.setAttribute('color', new THREE.BufferAttribute(colors, 3));

  const mat = new THREE.PointsMaterial({
    map: sprite,
    size: 0.18,
    sizeAttenuation: true,
    transparent: true,
    opacity: 0.35,
    vertexColors: true,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
  });

  return { points: new THREE.Points(geo, mat), geo, phases, velocities };
}

/**
 * Animated bind line from source → core that flashes on SSE events.
 */
export function makeBindLine(color: number, opacity: number): {
  line: THREE.Line;
  setEnds: (a: THREE.Vector3, b: THREE.Vector3) => void;
} {
  const geo = new THREE.BufferGeometry();
  const positions = new Float32Array(6); // 2 points × 3 components
  geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  const mat = new THREE.LineBasicMaterial({
    color, transparent: true, opacity,
    blending: THREE.AdditiveBlending,
  });
  const line = new THREE.Line(geo, mat);
  line.visible = false;

  function setEnds(a: THREE.Vector3, b: THREE.Vector3) {
    const arr = geo.attributes.position.array as Float32Array;
    arr[0] = a.x; arr[1] = a.y; arr[2] = a.z;
    arr[3] = b.x; arr[4] = b.y; arr[5] = b.z;
    geo.attributes.position.needsUpdate = true;
  }

  return { line, setEnds };
}

/**
 * Small particle burst that triggers on events. Returns updatable object.
 */
export function makeParticleBurst(
  count: number,
  color: number,
  sprite: THREE.Texture,
): {
  points: THREE.Points;
  trigger: (origin: THREE.Vector3) => void;
  update: (dt: number) => void;
} {
  const positions = new Float32Array(count * 3);
  const vels = new Float32Array(count * 3);
  let life = 0;

  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));

  const mat = new THREE.PointsMaterial({
    map: sprite,
    size: 0.12,
    transparent: true,
    opacity: 0,
    color,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
  });

  const points = new THREE.Points(geo, mat);

  function trigger(origin: THREE.Vector3) {
    life = 1.0;
    for (let i = 0; i < count; i++) {
      positions[i * 3] = origin.x;
      positions[i * 3 + 1] = origin.y;
      positions[i * 3 + 2] = origin.z;
      vels[i * 3] = (Math.random() - 0.5) * 2;
      vels[i * 3 + 1] = (Math.random() - 0.5) * 2;
      vels[i * 3 + 2] = (Math.random() - 0.5) * 2;
    }
    geo.attributes.position.needsUpdate = true;
    mat.opacity = 0.8;
  }

  function update(dt: number) {
    if (life <= 0) return;
    life -= dt * 1.5;
    mat.opacity = Math.max(0, life * 0.8);
    const arr = geo.attributes.position.array as Float32Array;
    for (let i = 0; i < count; i++) {
      arr[i * 3] += vels[i * 3] * dt;
      arr[i * 3 + 1] += vels[i * 3 + 1] * dt;
      arr[i * 3 + 2] += vels[i * 3 + 2] * dt;
    }
    geo.attributes.position.needsUpdate = true;
  }

  return { points, trigger, update };
}
