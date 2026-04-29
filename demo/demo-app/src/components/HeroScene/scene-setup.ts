import * as THREE from 'three';

export const TAU = Math.PI * 2;
export const DUST_COUNT = 280;

/* ── rosedust palette ── */
export const COL = {
  roseGlow: 0xdca5bd,
  roseBright: 0xcc90a8,
  roseDim: 0x7a5060,
  rose: 0xaa7088,
  bone: 0xc8b890,
  boneBright: 0xd8c8a0,
  core: 0xe8b5ce,
  dim: 0x443844,
  faint: 0x2a1e28,
};

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

/* ── helper: create a thin line ring ── */
export function makeLineRing(radius: number, segments: number, color: number, opacity: number): THREE.Line {
  const pts: THREE.Vector3[] = [];
  for (let i = 0; i <= segments; i++) {
    const a = (i / segments) * TAU;
    pts.push(new THREE.Vector3(Math.cos(a) * radius, 0, Math.sin(a) * radius));
  }
  const geo = new THREE.BufferGeometry().setFromPoints(pts);
  const mat = new THREE.LineBasicMaterial({ color, transparent: true, opacity });
  return new THREE.Line(geo, mat);
}

/* ── helper: create tick marks around a ring ── */
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

/* ── helper: arc segment ── */
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

/** All mutable scene objects needed by the animation loop */
export interface SceneObjects {
  primaryGroup: THREE.Group;
  secondaryGroup: THREE.Group;
  tertiaryGroup: THREE.Group;
  arcGroup: THREE.Group;
  coreL0: THREE.Mesh;
  coreL1: THREE.Mesh;
  coreL2: THREE.Mesh;
  nodeMeshes: THREE.Mesh[];
  nodeGlowMeshes: THREE.Mesh[];
  nodeOuterMeshes: THREE.Mesh[];
  tokens: THREE.Mesh[];
  tokenTrails: THREE.Mesh[][];
  tokenSpeeds: number[];
  tokenOffsets: number[];
  dustGeo: THREE.BufferGeometry;
  dustPhases: Float32Array;
  dustVelocities: Float32Array;
}

/** Build the entire Three.js scene graph and return handles for animation */
export function buildScene(scene: THREE.Scene, sprite: THREE.Texture): SceneObjects {
  const RING_R = 5.0;

  /* ════════════════════════════════════════
     ARMILLARY — nested rotating ring groups
     ════════════════════════════════════════ */
  const world = new THREE.Group();
  scene.add(world);

  /* ── primary ring (the 8-node loop) ── */
  const primaryGroup = new THREE.Group();
  world.add(primaryGroup);

  // Main ring line
  primaryGroup.add(makeLineRing(RING_R, 192, COL.roseDim, 0.35));
  // Tick marks (64 small ticks)
  primaryGroup.add(makeTickMarks(RING_R, 64, 0.08, COL.dim, 0.2));
  // Tick marks (8 major ticks at node positions)
  primaryGroup.add(makeTickMarks(RING_R, 8, 0.2, COL.rose, 0.4));

  /* ── 8 phase nodes ── */
  const nodeGeo = new THREE.OctahedronGeometry(0.3, 0);
  const nodeInnerGeo = new THREE.OctahedronGeometry(0.18, 0);
  const nodeOuterGeo = new THREE.OctahedronGeometry(0.42, 0);
  const nodeMeshes: THREE.Mesh[] = [];
  const nodeGlowMeshes: THREE.Mesh[] = [];
  const nodeOuterMeshes: THREE.Mesh[] = [];

  for (let i = 0; i < 8; i++) {
    const angle = (i / 8) * TAU - Math.PI / 2;
    const x = Math.cos(angle) * RING_R;
    const z = Math.sin(angle) * RING_R;

    // Outer cage (very faint, larger)
    const outerMat = new THREE.MeshBasicMaterial({
      color: COL.dim, wireframe: true, transparent: true, opacity: 0.15,
    });
    const outer = new THREE.Mesh(nodeOuterGeo, outerMat);
    outer.position.set(x, 0, z);
    primaryGroup.add(outer);
    nodeOuterMeshes.push(outer);

    // Wireframe node
    const wireMat = new THREE.MeshBasicMaterial({
      color: COL.rose, wireframe: true, transparent: true, opacity: 0.5,
    });
    const wire = new THREE.Mesh(nodeGeo, wireMat);
    wire.position.set(x, 0, z);
    primaryGroup.add(wire);
    nodeMeshes.push(wire);

    // Inner glow solid
    const glowMat = new THREE.MeshBasicMaterial({
      color: COL.roseGlow, transparent: true, opacity: 0.12,
    });
    const glow = new THREE.Mesh(nodeInnerGeo, glowMat);
    glow.position.set(x, 0, z);
    primaryGroup.add(glow);
    nodeGlowMeshes.push(glow);
  }

  /* ── spokes (thin dashed-feel lines) ── */
  for (let i = 0; i < 8; i++) {
    const angle = (i / 8) * TAU - Math.PI / 2;
    const x = Math.cos(angle) * RING_R;
    const z = Math.sin(angle) * RING_R;
    // Inner spoke (core to 40% radius)
    const inner = new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(0, 0, 0),
      new THREE.Vector3(x * 0.35, 0, z * 0.35),
    ]);
    primaryGroup.add(new THREE.Line(inner, new THREE.LineBasicMaterial({
      color: COL.dim, transparent: true, opacity: 0.12,
    })));
    // Outer spoke (60% to node)
    const outerSpoke = new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(x * 0.65, 0, z * 0.65),
      new THREE.Vector3(x, 0, z),
    ]);
    primaryGroup.add(new THREE.Line(outerSpoke, new THREE.LineBasicMaterial({
      color: COL.dim, transparent: true, opacity: 0.1,
    })));
  }

  /* ── central core — nested octahedra ── */
  const coreGroup = new THREE.Group();
  primaryGroup.add(coreGroup);

  const coreL0 = new THREE.Mesh(
    new THREE.OctahedronGeometry(0.7, 0),
    new THREE.MeshBasicMaterial({ color: COL.roseGlow, wireframe: true, transparent: true, opacity: 0.4 }),
  );
  coreGroup.add(coreL0);

  const coreL1 = new THREE.Mesh(
    new THREE.OctahedronGeometry(0.45, 0),
    new THREE.MeshBasicMaterial({ color: COL.core, wireframe: true, transparent: true, opacity: 0.35 }),
  );
  coreGroup.add(coreL1);

  const coreL2 = new THREE.Mesh(
    new THREE.OctahedronGeometry(0.25, 0),
    new THREE.MeshBasicMaterial({ color: COL.core, transparent: true, opacity: 0.2 }),
  );
  coreGroup.add(coreL2);

  /* ── secondary ring — tilted, slower ── */
  const secondaryGroup = new THREE.Group();
  secondaryGroup.rotation.x = Math.PI * 0.38;
  secondaryGroup.rotation.z = Math.PI * 0.12;
  world.add(secondaryGroup);
  secondaryGroup.add(makeLineRing(RING_R * 0.92, 128, COL.dim, 0.15));
  secondaryGroup.add(makeTickMarks(RING_R * 0.92, 32, 0.06, COL.faint, 0.12));

  /* ── tertiary ring — opposite tilt ── */
  const tertiaryGroup = new THREE.Group();
  tertiaryGroup.rotation.x = -Math.PI * 0.25;
  tertiaryGroup.rotation.z = -Math.PI * 0.08;
  world.add(tertiaryGroup);
  tertiaryGroup.add(makeLineRing(RING_R * 1.08, 128, COL.faint, 0.12));

  /* ── decorative arcs — NieR-style partial circles ── */
  const arcGroup = new THREE.Group();
  world.add(arcGroup);
  const arcConfigs = [
    { r: 3.2, start: 0.2, end: 1.4, tiltX: 0.6, tiltZ: 0.3, color: COL.roseDim, op: 0.18 },
    { r: 6.2, start: 2.8, end: 4.2, tiltX: -0.3, tiltZ: 0.15, color: COL.dim, op: 0.1 },
    { r: 2.0, start: 4.0, end: 5.5, tiltX: 0.9, tiltZ: -0.2, color: COL.faint, op: 0.15 },
    { r: 7.0, start: 0.5, end: 1.2, tiltX: 0.15, tiltZ: 0.4, color: COL.roseDim, op: 0.08 },
    { r: 4.0, start: 3.5, end: 5.0, tiltX: -0.7, tiltZ: 0.5, color: COL.dim, op: 0.12 },
  ];
  for (const cfg of arcConfigs) {
    const arc = makeArc(cfg.r, cfg.start, cfg.end, 48, cfg.color, cfg.op);
    arc.rotation.x = cfg.tiltX;
    arc.rotation.z = cfg.tiltZ;
    arcGroup.add(arc);
  }

  /* ── orbiting tokens — 3 glowing spheres ── */
  const tokenGeo = new THREE.SphereGeometry(0.08, 12, 12);
  const tokenTrailGeo = new THREE.SphereGeometry(0.04, 8, 8);
  const tokenColors = [COL.roseGlow, COL.bone, COL.roseBright];
  const tokenSpeeds = [0.35, 0.5, 0.25];
  const tokenOffsets = [0, TAU / 3, (2 * TAU) / 3];
  const tokens: THREE.Mesh[] = [];
  const tokenTrails: THREE.Mesh[][] = [];

  for (let i = 0; i < 3; i++) {
    const mat = new THREE.MeshBasicMaterial({
      color: tokenColors[i], transparent: true, opacity: 0.9,
    });
    const tok = new THREE.Mesh(tokenGeo, mat);
    primaryGroup.add(tok);
    tokens.push(tok);

    // Trail dots (4 per token)
    const trails: THREE.Mesh[] = [];
    for (let t = 0; t < 4; t++) {
      const trailMat = new THREE.MeshBasicMaterial({
        color: tokenColors[i], transparent: true, opacity: 0.3 - t * 0.06,
      });
      const trail = new THREE.Mesh(tokenTrailGeo, trailMat);
      primaryGroup.add(trail);
      trails.push(trail);
    }
    tokenTrails.push(trails);
  }

  /* ── ambient dust ── */
  const dustPositions = new Float32Array(DUST_COUNT * 3);
  const dustColors = new Float32Array(DUST_COUNT * 3);
  const dustPhases = new Float32Array(DUST_COUNT);
  const dustVelocities = new Float32Array(DUST_COUNT * 3);

  for (let i = 0; i < DUST_COUNT; i++) {
    const angle = Math.random() * TAU;
    const r = 1.0 + Math.random() * 9.0;
    const y = (Math.random() - 0.5) * 6;
    dustPositions[i * 3] = Math.cos(angle) * r;
    dustPositions[i * 3 + 1] = y;
    dustPositions[i * 3 + 2] = Math.sin(angle) * r;

    const roll = Math.random();
    const c = new THREE.Color(
      roll < 0.5 ? COL.faint : roll < 0.75 ? COL.roseDim : roll < 0.92 ? COL.rose : COL.bone,
    );
    dustColors[i * 3] = c.r;
    dustColors[i * 3 + 1] = c.g;
    dustColors[i * 3 + 2] = c.b;
    dustPhases[i] = Math.random() * TAU;
    dustVelocities[i * 3] = (Math.random() - 0.5) * 0.003;
    dustVelocities[i * 3 + 1] = (Math.random() - 0.5) * 0.002;
    dustVelocities[i * 3 + 2] = (Math.random() - 0.5) * 0.003;
  }

  const dustGeo = new THREE.BufferGeometry();
  dustGeo.setAttribute('position', new THREE.BufferAttribute(dustPositions, 3));
  dustGeo.setAttribute('color', new THREE.BufferAttribute(dustColors, 3));

  const dustMat = new THREE.PointsMaterial({
    map: sprite,
    size: 0.18,
    sizeAttenuation: true,
    transparent: true,
    opacity: 0.35,
    vertexColors: true,
    blending: THREE.AdditiveBlending,
    depthWrite: false,
  });
  world.add(new THREE.Points(dustGeo, dustMat));

  return {
    primaryGroup,
    secondaryGroup,
    tertiaryGroup,
    arcGroup,
    coreL0,
    coreL1,
    coreL2,
    nodeMeshes,
    nodeGlowMeshes,
    nodeOuterMeshes,
    tokens,
    tokenTrails,
    tokenSpeeds,
    tokenOffsets,
    dustGeo,
    dustPhases,
    dustVelocities,
  };
}
