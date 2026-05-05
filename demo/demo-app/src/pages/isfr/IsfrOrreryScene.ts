/**
 * ISFR Orrery — pure Three.js scene with orbital rings, source nodes, and ambient dust.
 *
 * No React. Called from IsfrDashboard via DashboardScene setup/animate callbacks.
 */
import * as THREE from 'three';
import { TAU, COL, CLASS_HEX, HEALTH_HEX } from '../../lib/three/constants';
import {
  makeLineRing, makeTickMarks, makeArc,
  makeTripleNode, makeDustField, makeBindLine, makeParticleBurst,
} from '../../lib/three/primitives';

/* ── Data interface (updated via refs from React) ─────────── */

export interface IsfrSceneData {
  compositeBps: number;
  confidenceBps: number;
  sources: Array<{
    name: string;
    class: string;
    health: string;
    rateBps: number;
    weight: number;
  }>;
  fieldDeltas: Record<string, number>;
}

/* ── Ring config per class ────────────────────────────────── */

const RING_CLASSES = ['lending', 'structured', 'staking', 'funding'] as const;
const RING_RADII = [3.5, 5.0, 6.5, 8.0];
const RING_TILTS: [number, number][] = [
  [0, 0],
  [0.08, 0.04],
  [-0.06, 0.06],
  [0.04, -0.05],
];

/* ── Scene objects ────────────────────────────────────────── */

interface SourceNode {
  name: string;
  cls: string;
  group: THREE.Group;
  wire: THREE.Mesh;
  solid: THREE.Mesh;
  halo: THREE.Mesh;
  angle: number;
  ringIdx: number;
}

interface BindLineEntry {
  name: string;
  line: THREE.Line;
  setEnds: (a: THREE.Vector3, b: THREE.Vector3) => void;
  life: number;
}

export interface IsfrSceneObjects {
  world: THREE.Group;
  coreGroup: THREE.Group;
  coreWire: THREE.Mesh;
  coreSolid: THREE.Mesh;
  coreHalo: THREE.Mesh;
  ringGroups: THREE.Group[];
  sourceNodes: SourceNode[];
  tokens: THREE.Mesh[];
  tokenSpeeds: number[];
  tokenOffsets: number[];
  dustGeo: THREE.BufferGeometry;
  dustPhases: Float32Array;
  dustVelocities: Float32Array;
  bindLines: BindLineEntry[];
  burst: ReturnType<typeof makeParticleBurst>;
  arcGroup: THREE.Group;
  flashSourceId: (id: string) => void;
}

/* ── Build ────────────────────────────────────────────────── */

export function buildIsfrScene(
  scene: THREE.Scene,
  sprite: THREE.Texture,
): IsfrSceneObjects {
  const world = new THREE.Group();
  scene.add(world);

  /* ── Central core ── */
  const coreNode = makeTripleNode(0.7, COL.roseGlow, COL.core);
  const coreGroup = coreNode.group;
  world.add(coreGroup);

  /* ── 4 orbital rings ── */
  const ringGroups: THREE.Group[] = [];
  for (let ri = 0; ri < 4; ri++) {
    const g = new THREE.Group();
    g.rotation.x = RING_TILTS[ri][0];
    g.rotation.z = RING_TILTS[ri][1];
    world.add(g);
    ringGroups.push(g);

    const color = CLASS_HEX[RING_CLASSES[ri]] ?? COL.rose;
    g.add(makeLineRing(RING_RADII[ri], 128, color, 0.3));
    g.add(makeTickMarks(RING_RADII[ri], 32, 0.06, color, 0.15));

    // Radial spokes (8 per ring)
    for (let s = 0; s < 8; s++) {
      const a = (s / 8) * TAU;
      const r = RING_RADII[ri];
      const cx = Math.cos(a);
      const cz = Math.sin(a);
      const spoke = new THREE.BufferGeometry().setFromPoints([
        new THREE.Vector3(cx * r * 0.15, 0, cz * r * 0.15),
        new THREE.Vector3(cx * r, 0, cz * r),
      ]);
      g.add(new THREE.Line(spoke, new THREE.LineBasicMaterial({
        color: COL.dim, transparent: true, opacity: 0.08,
      })));
    }
  }

  /* ── Orbit tokens (one per ring) ── */
  const tokenGeo = new THREE.SphereGeometry(0.06, 8, 8);
  const tokens: THREE.Mesh[] = [];
  const tokenSpeeds = [0.35, 0.5, 0.25, 0.4];
  const tokenOffsets = [0, TAU * 0.25, TAU * 0.5, TAU * 0.75];

  for (let i = 0; i < 4; i++) {
    const color = CLASS_HEX[RING_CLASSES[i]] ?? COL.rose;
    const tok = new THREE.Mesh(tokenGeo, new THREE.MeshBasicMaterial({
      color, transparent: true, opacity: 0.9,
    }));
    ringGroups[i].add(tok);
    tokens.push(tok);
  }

  /* ── Decorative arcs ── */
  const arcGroup = new THREE.Group();
  world.add(arcGroup);
  const arcConfigs = [
    { r: 2.5, start: 0.2, end: 1.4, tx: 0.6, tz: 0.3, c: COL.roseDim, o: 0.15 },
    { r: 9.5, start: 2.8, end: 4.2, tx: -0.3, tz: 0.15, c: COL.dim, o: 0.08 },
    { r: 1.8, start: 4.0, end: 5.5, tx: 0.9, tz: -0.2, c: COL.faint, o: 0.12 },
    { r: 10.0, start: 0.5, end: 1.2, tx: 0.15, tz: 0.4, c: COL.roseDim, o: 0.06 },
    { r: 4.0, start: 3.5, end: 5.0, tx: -0.7, tz: 0.5, c: COL.dim, o: 0.10 },
  ];
  for (const cfg of arcConfigs) {
    const arc = makeArc(cfg.r, cfg.start, cfg.end, 48, cfg.c, cfg.o);
    arc.rotation.x = cfg.tx;
    arc.rotation.z = cfg.tz;
    arcGroup.add(arc);
  }

  /* ── Ambient dust ── */
  const dust = makeDustField(200, 9, sprite);
  world.add(dust.points);

  /* ── Source nodes (populated later via data) ── */
  const sourceNodes: SourceNode[] = [];

  /* ── Bind lines + burst (for event flashes) ── */
  const bindLines: BindLineEntry[] = [];
  const burst = makeParticleBurst(24, COL.roseGlow, sprite);
  world.add(burst.points);

  const flashSourceId = (id: string) => {
    const bl = bindLines.find((b) => b.name === id);
    if (bl) {
      bl.life = 1.0;
      bl.line.visible = true;
      const sn = sourceNodes.find((n) => n.name === id);
      if (sn) burst.trigger(sn.group.position);
    }
  };

  return {
    world, coreGroup,
    coreWire: coreNode.wire, coreSolid: coreNode.solid, coreHalo: coreNode.halo,
    ringGroups, sourceNodes, tokens, tokenSpeeds, tokenOffsets,
    dustGeo: dust.geo, dustPhases: dust.phases, dustVelocities: dust.velocities,
    bindLines, burst, arcGroup, flashSourceId,
  };
}

/* ── Animate ──────────────────────────────────────────────── */

export function animateIsfrScene(
  objs: IsfrSceneObjects, dt: number, time: number,
  _mouse: { x: number; y: number }, data: IsfrSceneData,
): void {
  const coreScale = 0.5 + (data.compositeBps / 1000) * 0.5;
  const breathSpeed = 0.8 + (data.confidenceBps / 10000) * 0.8;
  const breath = coreScale * (1 + Math.sin(time * breathSpeed) * 0.12);
  objs.coreGroup.scale.setScalar(breath);
  objs.coreWire.rotation.y = time * 0.3;
  objs.coreWire.rotation.x = time * 0.2;
  objs.coreSolid.rotation.y = -time * 0.4;
  objs.coreHalo.rotation.y = time * 0.15;
  objs.coreHalo.rotation.z = time * 0.1;
  (objs.coreSolid.material as THREE.MeshBasicMaterial).opacity =
    0.15 + Math.sin(time * 1.5) * 0.08;

  for (let i = 0; i < objs.ringGroups.length; i++) {
    objs.ringGroups[i].rotation.y = time * (0.03 + i * 0.01) * (i % 2 === 0 ? 1 : -1);
  }

  for (let i = 0; i < 4; i++) {
    const delta = data.fieldDeltas[RING_CLASSES[i]] ?? 0;
    const speed = objs.tokenSpeeds[i] + Math.abs(delta) * 0.0005;
    const angle = time * speed + objs.tokenOffsets[i];
    const r = RING_RADII[i];
    objs.tokens[i].position.set(
      Math.cos(angle) * r, Math.sin(time * 1.2 + objs.tokenOffsets[i]) * 0.15, Math.sin(angle) * r,
    );
  }

  objs.arcGroup.rotation.y = time * 0.02;
  objs.arcGroup.rotation.x = Math.sin(time * 0.15) * 0.04;

  syncSourceNodes(objs, data, time);

  for (const bl of objs.bindLines) {
    if (bl.life > 0) {
      bl.life -= dt * 2.0;
      (bl.line.material as THREE.LineBasicMaterial).opacity = bl.life * 0.8;
      if (bl.life <= 0) bl.line.visible = false;
    }
  }
  objs.burst.update(dt);

  const posArr = objs.dustGeo.attributes.position.array as Float32Array;
  const dustCount = posArr.length / 3;
  for (let i = 0; i < dustCount; i++) {
    const ix = i * 3;
    posArr[ix] += objs.dustVelocities[ix];
    posArr[ix + 1] += objs.dustVelocities[ix + 1] + Math.sin(time + objs.dustPhases[i]) * 0.001;
    posArr[ix + 2] += objs.dustVelocities[ix + 2];
    if (posArr[ix] > 10) posArr[ix] = -10; if (posArr[ix] < -10) posArr[ix] = 10;
    if (posArr[ix + 1] > 5) posArr[ix + 1] = -5; if (posArr[ix + 1] < -5) posArr[ix + 1] = 5;
    if (posArr[ix + 2] > 10) posArr[ix + 2] = -10; if (posArr[ix + 2] < -10) posArr[ix + 2] = 10;
  }
  objs.dustGeo.attributes.position.needsUpdate = true;
}

function syncSourceNodes(objs: IsfrSceneObjects, data: IsfrSceneData, time: number): void {
  const currentNames = new Set(data.sources.map((s) => s.name));
  for (let i = objs.sourceNodes.length - 1; i >= 0; i--) {
    if (!currentNames.has(objs.sourceNodes[i].name)) {
      const sn = objs.sourceNodes[i];
      objs.ringGroups[sn.ringIdx]?.remove(sn.group);
      objs.sourceNodes.splice(i, 1);
      const blIdx = objs.bindLines.findIndex((b) => b.name === sn.name);
      if (blIdx >= 0) { objs.world.remove(objs.bindLines[blIdx].line); objs.bindLines.splice(blIdx, 1); }
    }
  }

  const classCounts: Record<string, number> = {};
  const classTotal: Record<string, number> = {};
  for (const s of data.sources) classTotal[s.class] = (classTotal[s.class] ?? 0) + 1;

  for (const s of data.sources) {
    classCounts[s.class] = (classCounts[s.class] ?? 0) + 1;
    const existing = objs.sourceNodes.find((n) => n.name === s.name);
    if (existing) {
      const hc = HEALTH_HEX[s.health] ?? COL.rose;
      (existing.wire.material as THREE.MeshBasicMaterial).color.setHex(hc);
      existing.group.scale.setScalar(0.3 + s.weight * 1.2);
      if (s.health === 'live') {
        existing.wire.scale.setScalar(1 + Math.sin(time * 3.0 + existing.angle) * 0.1);
        (existing.solid.material as THREE.MeshBasicMaterial).opacity = 0.25;
      } else if (s.health === 'stale') {
        (existing.solid.material as THREE.MeshBasicMaterial).opacity = 0.1;
        existing.wire.scale.setScalar(1);
      } else {
        (existing.solid.material as THREE.MeshBasicMaterial).opacity = 0.05;
        (existing.wire.material as THREE.MeshBasicMaterial).opacity = 0.2;
        existing.wire.scale.setScalar(1);
      }
      existing.wire.rotation.y = time * 0.4 + existing.angle;
      existing.wire.rotation.x = time * 0.25;
      existing.halo.rotation.y = time * 0.15;
    } else {
      const ringIdx = RING_CLASSES.indexOf(s.class as typeof RING_CLASSES[number]);
      const ri = ringIdx >= 0 ? ringIdx : 0;
      const total = classTotal[s.class] ?? 1;
      const idx = classCounts[s.class] ?? 1;
      const angle = ((idx - 1) / total) * TAU;
      const r = RING_RADII[ri];
      const nodeColor = HEALTH_HEX[s.health] ?? COL.rose;
      const node = makeTripleNode(0.25, nodeColor, nodeColor);
      node.group.position.set(Math.cos(angle) * r, 0, Math.sin(angle) * r);
      node.group.scale.setScalar(0.3 + s.weight * 1.2);
      objs.ringGroups[ri].add(node.group);
      objs.sourceNodes.push({ name: s.name, cls: s.class, ...node, angle, ringIdx: ri });
      const bl = makeBindLine(nodeColor, 0);
      objs.world.add(bl.line);
      bl.setEnds(new THREE.Vector3(0, 0, 0), node.group.position);
      objs.bindLines.push({ name: s.name, ...bl, life: 0 });
    }
  }
}
