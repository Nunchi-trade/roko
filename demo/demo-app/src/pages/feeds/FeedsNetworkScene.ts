/**
 * Feeds Network — force-directed 3D network graph.
 *
 * No React. Called from FeedsDashboard via DashboardScene callbacks.
 */
import * as THREE from 'three';
import { TAU, COL, FEED_KIND_HEX } from '../../lib/three/constants';
import {
  makeLineRing, makeTripleNode, makeDustField,
} from '../../lib/three/primitives';

/* ── Data interface ───────────────────────────────────────── */

export interface FeedsSceneData {
  agents: Array<{ agentId: string; name: string; online: boolean; feedIds: string[] }>;
  feeds: Array<{ feedId: string; name: string; kind: string; status: string; publisherAgentId: string }>;
}

/* ── Internal node types ──────────────────────────────────── */

interface AgentNode {
  id: string;
  group: THREE.Group;
  wire: THREE.Mesh;
  solid: THREE.Mesh;
  halo: THREE.Mesh;
  x: number; y: number; z: number;
  vx: number; vy: number; vz: number;
  online: boolean;
}

interface FeedNode {
  id: string;
  mesh: THREE.Mesh;
  publisherId: string;
  kind: string;
  x: number; y: number; z: number;
  vx: number; vy: number; vz: number;
}

interface ConnectionLine {
  agentId: string;
  feedId: string;
  line: THREE.Line;
  life: number; // flash intensity 0-1
}

interface FlowParticle {
  mesh: THREE.Mesh;
  agentId: string;
  feedId: string;
  t: number; // 0-1 progress along connection
  speed: number;
  active: boolean;
}

export interface FeedsSceneObjects {
  world: THREE.Group;
  agentNodes: AgentNode[];
  feedNodes: FeedNode[];
  connections: ConnectionLine[];
  particles: FlowParticle[];
  dustGeo: THREE.BufferGeometry;
  dustPhases: Float32Array;
  dustVelocities: Float32Array;
  converged: boolean;
  flashFeedId: (id: string) => void;
  updateLayout: (data: FeedsSceneData) => void;
}

/* ── Build ────────────────────────────────────────────────── */

export function buildFeedsScene(
  scene: THREE.Scene,
  sprite: THREE.Texture,
): FeedsSceneObjects {
  const world = new THREE.Group();
  scene.add(world);

  // Decorative framing rings
  world.add(makeLineRing(12, 128, COL.dim, 0.1));
  world.add(makeLineRing(15, 128, COL.faint, 0.06));

  // Ambient dust (teal tinted)
  const dust = makeDustField(150, 12, sprite, [COL.teal, COL.dim, COL.faint, COL.tealBright]);
  world.add(dust.points);

  const objs: FeedsSceneObjects = {
    world,
    agentNodes: [],
    feedNodes: [],
    connections: [],
    particles: [],
    dustGeo: dust.geo,
    dustPhases: dust.phases,
    dustVelocities: dust.velocities,
    converged: false,
    flashFeedId: () => {},
    updateLayout: () => {},
  };

  objs.flashFeedId = (id: string) => {
    for (const conn of objs.connections) {
      if (conn.feedId === id) {
        conn.life = 1.0;
        // Spawn a flow particle
        const idle = objs.particles.find((p) => !p.active);
        if (idle) {
          idle.agentId = conn.agentId;
          idle.feedId = conn.feedId;
          idle.t = 0;
          idle.speed = 0.8 + Math.random() * 0.6;
          idle.active = true;
          idle.mesh.visible = true;
          const feed = objs.feedNodes.find((f) => f.id === id);
          if (feed) {
            const kindColor = FEED_KIND_HEX[feed.kind] ?? COL.teal;
            (idle.mesh.material as THREE.MeshBasicMaterial).color.setHex(kindColor);
          }
        }
      }
    }
  };

  objs.updateLayout = (data: FeedsSceneData) => {
    rebuildGraph(objs, data);
  };

  // Pre-allocate flow particles
  const particleGeo = new THREE.SphereGeometry(0.06, 6, 6);
  for (let i = 0; i < 30; i++) {
    const mesh = new THREE.Mesh(particleGeo, new THREE.MeshBasicMaterial({
      color: COL.teal, transparent: true, opacity: 0.8,
      blending: THREE.AdditiveBlending,
    }));
    mesh.visible = false;
    world.add(mesh);
    objs.particles.push({
      mesh, agentId: '', feedId: '', t: 0, speed: 1, active: false,
    });
  }

  return objs;
}

/* ── Rebuild graph when agents/feeds change ───────────────── */

function rebuildGraph(objs: FeedsSceneObjects, data: FeedsSceneData): void {
  // Remove old nodes
  for (const an of objs.agentNodes) objs.world.remove(an.group);
  for (const fn of objs.feedNodes) objs.world.remove(fn.mesh);
  for (const conn of objs.connections) objs.world.remove(conn.line);
  objs.agentNodes.length = 0;
  objs.feedNodes.length = 0;
  objs.connections.length = 0;

  // Create agent nodes
  for (let i = 0; i < data.agents.length; i++) {
    const agent = data.agents[i];
    const angle = (i / Math.max(data.agents.length, 1)) * TAU;
    const r = 4 + Math.random() * 2;
    const color = agent.online ? COL.teal : COL.roseDim;
    const node = makeTripleNode(0.5, color, agent.online ? COL.tealBright : COL.dim);
    const x = Math.cos(angle) * r;
    const z = Math.sin(angle) * r;
    node.group.position.set(x, 0, z);
    objs.world.add(node.group);
    objs.agentNodes.push({
      id: agent.agentId, ...node,
      x, y: 0, z, vx: 0, vy: 0, vz: 0, online: agent.online,
    });
  }

  // Create feed nodes
  for (let i = 0; i < data.feeds.length; i++) {
    const feed = data.feeds[i];
    const publisher = objs.agentNodes.find((a) => a.id === feed.publisherAgentId);
    const bx = publisher ? publisher.x : 0;
    const bz = publisher ? publisher.z : 0;
    const offset = (i * 0.7);
    const x = bx + Math.cos(offset) * 1.5;
    const z = bz + Math.sin(offset) * 1.5;

    const kindColor = FEED_KIND_HEX[feed.kind] ?? COL.teal;
    const geo = new THREE.IcosahedronGeometry(0.2, 0);
    const mesh = new THREE.Mesh(geo, new THREE.MeshBasicMaterial({
      color: kindColor, transparent: true, opacity: feed.status === 'live' ? 0.8 : 0.3,
      wireframe: true,
    }));
    mesh.position.set(x, 0.3, z);
    objs.world.add(mesh);
    objs.feedNodes.push({
      id: feed.feedId, mesh, publisherId: feed.publisherAgentId,
      kind: feed.kind, x, y: 0.3, z, vx: 0, vy: 0, vz: 0,
    });

    // Connection line
    if (publisher) {
      const geo = new THREE.BufferGeometry().setFromPoints([
        new THREE.Vector3(publisher.x, 0, publisher.z),
        new THREE.Vector3(x, 0.3, z),
      ]);
      const line = new THREE.Line(geo, new THREE.LineBasicMaterial({
        color: kindColor, transparent: true, opacity: 0.2,
      }));
      objs.world.add(line);
      objs.connections.push({ agentId: publisher.id, feedId: feed.feedId, line, life: 0 });
    }
  }

  objs.converged = false;
}

/* ── Animate ──────────────────────────────────────────────── */

export function animateFeedsScene(
  objs: FeedsSceneObjects, dt: number, time: number,
  _mouse: { x: number; y: number }, _data: FeedsSceneData,
): void {
  // Force layout iterations (simple repulsion + spring)
  if (!objs.converged) {
    let maxDelta = 0;
    const allNodes = [
      ...objs.agentNodes.map((n) => ({ n, mass: 2 })),
      ...objs.feedNodes.map((n) => ({ n, mass: 0.5 })),
    ];

    for (let iter = 0; iter < 3; iter++) {
      // Repulsion between all pairs
      for (let i = 0; i < allNodes.length; i++) {
        for (let j = i + 1; j < allNodes.length; j++) {
          const a = allNodes[i].n;
          const b = allNodes[j].n;
          const dx = a.x - b.x;
          const dz = a.z - b.z;
          const dist = Math.sqrt(dx * dx + dz * dz) + 0.1;
          const force = 3.0 / (dist * dist);
          const fx = (dx / dist) * force;
          const fz = (dz / dist) * force;
          a.vx += fx * 0.1; a.vz += fz * 0.1;
          b.vx -= fx * 0.1; b.vz -= fz * 0.1;
        }
      }

      // Springs: feed → publisher
      for (const fn of objs.feedNodes) {
        const pub = objs.agentNodes.find((a) => a.id === fn.publisherId);
        if (!pub) continue;
        const dx = fn.x - pub.x;
        const dz = fn.z - pub.z;
        const dist = Math.sqrt(dx * dx + dz * dz);
        const ideal = 2.0;
        const force = (dist - ideal) * 0.05;
        const fx = (dx / (dist + 0.01)) * force;
        const fz = (dz / (dist + 0.01)) * force;
        fn.vx -= fx; fn.vz -= fz;
        pub.vx += fx * 0.3; pub.vz += fz * 0.3;
      }

      // Apply velocities with damping
      for (const { n } of allNodes) {
        n.vx *= 0.85; n.vz *= 0.85;
        n.x += n.vx * 0.1;
        n.z += n.vz * 0.1;
        maxDelta = Math.max(maxDelta, Math.abs(n.vx), Math.abs(n.vz));
      }
    }

    // Update positions
    for (const an of objs.agentNodes) {
      an.group.position.x = an.x;
      an.group.position.z = an.z;
    }
    for (const fn of objs.feedNodes) {
      fn.mesh.position.x = fn.x;
      fn.mesh.position.z = fn.z;
    }

    // Update connection lines
    for (const conn of objs.connections) {
      const agent = objs.agentNodes.find((a) => a.id === conn.agentId);
      const feed = objs.feedNodes.find((f) => f.id === conn.feedId);
      if (agent && feed) {
        const arr = conn.line.geometry.attributes.position.array as Float32Array;
        arr[0] = agent.x; arr[1] = 0; arr[2] = agent.z;
        arr[3] = feed.x; arr[4] = feed.y; arr[5] = feed.z;
        conn.line.geometry.attributes.position.needsUpdate = true;
      }
    }

    if (maxDelta < 0.01) objs.converged = true;
  }

  // Agent node animation
  for (const an of objs.agentNodes) {
    an.wire.rotation.y = time * 0.3;
    an.wire.rotation.x = time * 0.2;
    an.halo.rotation.y = time * 0.15;
    if (an.online) {
      const pulse = 1 + Math.sin(time * 2.0) * 0.05;
      an.group.scale.setScalar(pulse);
      (an.solid.material as THREE.MeshBasicMaterial).opacity = 0.2;
    } else {
      an.group.scale.setScalar(0.8);
      (an.solid.material as THREE.MeshBasicMaterial).opacity = 0.05;
    }
  }

  // Feed node rotation
  for (const fn of objs.feedNodes) {
    fn.mesh.rotation.y = time * 0.5;
    fn.mesh.rotation.x = time * 0.3;
  }

  // Connection flash fade
  for (const conn of objs.connections) {
    if (conn.life > 0) {
      conn.life -= dt * 2;
      (conn.line.material as THREE.LineBasicMaterial).opacity = 0.2 + conn.life * 0.6;
    } else {
      (conn.line.material as THREE.LineBasicMaterial).opacity = 0.2;
    }
  }

  // Flow particles
  for (const p of objs.particles) {
    if (!p.active) continue;
    p.t += dt * p.speed;
    if (p.t >= 1) {
      p.active = false;
      p.mesh.visible = false;
      continue;
    }
    const agent = objs.agentNodes.find((a) => a.id === p.agentId);
    const feed = objs.feedNodes.find((f) => f.id === p.feedId);
    if (agent && feed) {
      p.mesh.position.lerpVectors(
        new THREE.Vector3(agent.x, 0, agent.z),
        new THREE.Vector3(feed.x, feed.y, feed.z),
        p.t,
      );
      (p.mesh.material as THREE.MeshBasicMaterial).opacity = 0.8 * (1 - p.t * 0.5);
    }
  }

  // Dust drift
  const posArr = objs.dustGeo.attributes.position.array as Float32Array;
  const dustCount = posArr.length / 3;
  for (let i = 0; i < dustCount; i++) {
    const ix = i * 3;
    posArr[ix] += objs.dustVelocities[ix];
    posArr[ix + 1] += objs.dustVelocities[ix + 1] + Math.sin(time + objs.dustPhases[i]) * 0.001;
    posArr[ix + 2] += objs.dustVelocities[ix + 2];
    if (posArr[ix] > 12) posArr[ix] = -12; if (posArr[ix] < -12) posArr[ix] = 12;
    if (posArr[ix + 1] > 5) posArr[ix + 1] = -5; if (posArr[ix + 1] < -5) posArr[ix + 1] = 5;
    if (posArr[ix + 2] > 12) posArr[ix + 2] = -12; if (posArr[ix + 2] < -12) posArr[ix + 2] = 12;
  }
  objs.dustGeo.attributes.position.needsUpdate = true;
}
