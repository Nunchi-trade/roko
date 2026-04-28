import { useRef, useEffect, useMemo } from 'react';
import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';
import { useThreeScene } from '../lib/three-shared';

interface KnowledgeEntry {
  id: string;
  domain?: string;
  citations?: number;
  label?: string;
}

interface KnowledgeEdge {
  source: string;
  target: string;
  frequency?: number;
}

interface Props {
  entries: KnowledgeEntry[];
  edges: KnowledgeEdge[];
}

const DOMAIN_COLORS: Record<string, number> = {
  agent: 0xb97894,
  gate: 0x7d9e8c,
  plan: 0xc39b5f,
  knowledge: 0x7873a5,
  config: 0x6a9ea0,
};

const DEFAULT_COLOR = 0x988090;

interface SimNode {
  x: number;
  y: number;
  z: number;
  vx: number;
  vy: number;
  vz: number;
  mesh: THREE.Mesh;
  glow: THREE.Sprite;
  entry: KnowledgeEntry;
}

function createGlowTexture(): THREE.Texture {
  const size = 64;
  const canvas = document.createElement('canvas');
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext('2d')!;
  const gradient = ctx.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2);
  gradient.addColorStop(0, 'rgba(255,255,255,0.5)');
  gradient.addColorStop(0.4, 'rgba(255,255,255,0.15)');
  gradient.addColorStop(1, 'rgba(255,255,255,0)');
  ctx.fillStyle = gradient;
  ctx.fillRect(0, 0, size, size);
  const tex = new THREE.CanvasTexture(canvas);
  return tex;
}

function nodeRadius(citations: number): number {
  return Math.min(0.6, Math.max(0.15, citations * 0.08));
}

export default function KnowledgeGraph3D({ entries, edges }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const three = useThreeScene(containerRef, { cameraZ: 12, cameraFov: 50 });
  const hoveredRef = useRef<SimNode | null>(null);
  const nodesRef = useRef<SimNode[]>([]);
  const edgePairsRef = useRef<[number, number][]>([]);
  const lineRef = useRef<THREE.LineSegments | null>(null);
  const controlsRef = useRef<OrbitControls | null>(null);

  const glowTexture = useMemo(() => createGlowTexture(), []);

  // Build scene objects when entries/edges change
  useEffect(() => {
    if (!three) return;
    const { scene } = three;

    // Clear previous
    while (scene.children.length > 0) scene.remove(scene.children[0]);

    // Empty state
    if (entries.length === 0) {
      const wireGeo = new THREE.IcosahedronGeometry(1.5, 1);
      const wireMat = new THREE.MeshBasicMaterial({ color: 0x3a303a, wireframe: true });
      const wireMesh = new THREE.Mesh(wireGeo, wireMat);
      scene.add(wireMesh);
      const animate = () => {
        wireMesh.rotation.y += 0.003;
        wireMesh.rotation.x += 0.001;
      };
      const emptyId = { value: 0 };
      const loop = () => {
        animate();
        three.renderer.render(scene, three.camera);
        emptyId.value = requestAnimationFrame(loop);
      };
      emptyId.value = requestAnimationFrame(loop);
      return () => cancelAnimationFrame(emptyId.value);
    }

    const idToIdx = new Map(entries.map((e, i) => [e.id, i]));

    // Build sim nodes
    const simNodes: SimNode[] = entries.map((entry) => {
      const r = nodeRadius(entry.citations ?? 1);
      const color = DOMAIN_COLORS[entry.domain ?? ''] ?? DEFAULT_COLOR;

      const geo = new THREE.SphereGeometry(r, 16, 12);
      const mat = new THREE.MeshBasicMaterial({ color, transparent: true, opacity: 0.85 });
      const mesh = new THREE.Mesh(geo, mat);
      mesh.position.set(
        (Math.random() - 0.5) * 8,
        (Math.random() - 0.5) * 8,
        (Math.random() - 0.5) * 8,
      );
      scene.add(mesh);

      const spriteMat = new THREE.SpriteMaterial({
        map: glowTexture,
        color,
        transparent: true,
        opacity: 0.4,
        blending: THREE.AdditiveBlending,
        depthWrite: false,
      });
      const glow = new THREE.Sprite(spriteMat);
      glow.scale.set(r * 4, r * 4, 1);
      mesh.add(glow);

      return {
        x: mesh.position.x,
        y: mesh.position.y,
        z: mesh.position.z,
        vx: 0, vy: 0, vz: 0,
        mesh,
        glow,
        entry,
      };
    });
    nodesRef.current = simNodes;

    // Build edge pairs
    const pairs: [number, number][] = [];
    for (const edge of edges) {
      const si = idToIdx.get(edge.source);
      const ti = idToIdx.get(edge.target);
      if (si != null && ti != null) pairs.push([si, ti]);
    }
    edgePairsRef.current = pairs;

    // Build edge line segments
    const positions = new Float32Array(pairs.length * 6);
    const lineGeo = new THREE.BufferGeometry();
    lineGeo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
    const lineMat = new THREE.LineBasicMaterial({
      color: 0xa58e9e,
      transparent: true,
      opacity: 0.2,
    });
    const lineSegs = new THREE.LineSegments(lineGeo, lineMat);
    scene.add(lineSegs);
    lineRef.current = lineSegs;
  }, [three, entries, edges, glowTexture]);

  // Animation loop with force simulation
  useEffect(() => {
    if (!three) return;
    const { scene, camera, renderer } = three;

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.dampingFactor = 0.08;
    controls.enablePan = false;
    controls.minDistance = 3;
    controls.maxDistance = 30;
    controlsRef.current = controls;

    const raycaster = new THREE.Raycaster();
    const mouse = new THREE.Vector2(9999, 9999);

    const onMouseMove = (e: MouseEvent) => {
      const rect = renderer.domElement.getBoundingClientRect();
      mouse.x = ((e.clientX - rect.left) / rect.width) * 2 - 1;
      mouse.y = -((e.clientY - rect.top) / rect.height) * 2 + 1;
    };
    renderer.domElement.addEventListener('mousemove', onMouseMove);

    let animId = 0;
    const repulsion = 2.0;
    const attraction = 0.008;
    const centerStrength = 0.002;
    const damping = 0.92;

    const loop = () => {
      const nodes = nodesRef.current;
      const pairs = edgePairsRef.current;

      if (nodes.length > 0) {
        // Force simulation step
        for (let i = 0; i < nodes.length; i++) {
          for (let j = i + 1; j < nodes.length; j++) {
            let dx = nodes[j].x - nodes[i].x;
            let dy = nodes[j].y - nodes[i].y;
            let dz = nodes[j].z - nodes[i].z;
            const dist = Math.sqrt(dx * dx + dy * dy + dz * dz) || 0.1;
            const force = repulsion / (dist * dist);
            dx = (dx / dist) * force;
            dy = (dy / dist) * force;
            dz = (dz / dist) * force;
            nodes[i].vx -= dx; nodes[i].vy -= dy; nodes[i].vz -= dz;
            nodes[j].vx += dx; nodes[j].vy += dy; nodes[j].vz += dz;
          }
        }

        for (const [si, ti] of pairs) {
          const dx = nodes[ti].x - nodes[si].x;
          const dy = nodes[ti].y - nodes[si].y;
          const dz = nodes[ti].z - nodes[si].z;
          nodes[si].vx += dx * attraction;
          nodes[si].vy += dy * attraction;
          nodes[si].vz += dz * attraction;
          nodes[ti].vx -= dx * attraction;
          nodes[ti].vy -= dy * attraction;
          nodes[ti].vz -= dz * attraction;
        }

        for (const node of nodes) {
          node.vx -= node.x * centerStrength;
          node.vy -= node.y * centerStrength;
          node.vz -= node.z * centerStrength;
          node.vx *= damping;
          node.vy *= damping;
          node.vz *= damping;
          node.x += node.vx;
          node.y += node.vy;
          node.z += node.vz;
          node.mesh.position.set(node.x, node.y, node.z);
        }

        // Update edge positions
        const line = lineRef.current;
        if (line) {
          const posAttr = line.geometry.getAttribute('position') as THREE.BufferAttribute;
          for (let i = 0; i < pairs.length; i++) {
            const [si, ti] = pairs[i];
            posAttr.setXYZ(i * 2, nodes[si].x, nodes[si].y, nodes[si].z);
            posAttr.setXYZ(i * 2 + 1, nodes[ti].x, nodes[ti].y, nodes[ti].z);
          }
          posAttr.needsUpdate = true;
        }

        // Hover raycast
        raycaster.setFromCamera(mouse, camera);
        const meshes = nodes.map((n) => n.mesh);
        const hits = raycaster.intersectObjects(meshes);
        const prev = hoveredRef.current;
        if (prev) {
          prev.mesh.scale.set(1, 1, 1);
          hoveredRef.current = null;
        }
        if (hits.length > 0) {
          const hit = nodes.find((n) => n.mesh === hits[0].object);
          if (hit) {
            hit.mesh.scale.set(1.5, 1.5, 1.5);
            hoveredRef.current = hit;
          }
        }
      }

      controls.update();
      renderer.render(scene, camera);
      animId = requestAnimationFrame(loop);
    };
    animId = requestAnimationFrame(loop);

    return () => {
      cancelAnimationFrame(animId);
      renderer.domElement.removeEventListener('mousemove', onMouseMove);
      controls.dispose();
    };
  }, [three]);

  return (
    <div style={{ position: 'relative', width: '100%', height: 'calc(100vh - 200px)' }}>
      <div
        ref={containerRef}
        style={{ width: '100%', height: '100%', background: 'rgba(6,6,8,.3)', borderRadius: 2 }}
      />
      {entries.length === 0 && (
        <div style={{
          position: 'absolute', top: '50%', left: '50%',
          transform: 'translate(-50%, -50%)',
          fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--text-dim, #6a5a68)',
          textAlign: 'center', pointerEvents: 'none',
        }}>
          No knowledge entries
        </div>
      )}
      <div style={{
        position: 'absolute', top: 10, left: 10,
        fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--text-dim, #6a5a68)',
        pointerEvents: 'none',
      }}>
        <div>{entries.length} nodes</div>
        <div>{edges.length} edges</div>
      </div>
    </div>
  );
}
