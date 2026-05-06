import { useEffect, useRef, type RefObject } from 'react';
import * as THREE from 'three';
import { useThreeScene } from '../../lib/three-shared';
import { useIntersectionGate } from './SceneFactory';

/* ── Types ── */

export interface KnowledgeNode {
  id: string;
  label: string;
  domain: 'gate' | 'agent' | 'knowledge' | 'plan' | 'config';
  weight?: number;
}

export interface KnowledgeEdge {
  source: string;
  target: string;
  strength?: number;
}

interface KnowledgeOrbitProps {
  nodes: KnowledgeNode[];
  edges: KnowledgeEdge[];
  height?: number;
}

/* ── Domain colors (hex ints for THREE) ── */

const DOMAIN_HEX: Record<string, number> = {
  gate:      0x2dd4bf,
  agent:     0xcc90a8,
  knowledge: 0xc8b890,
  plan:      0xa864b4,
  config:    0x8888a8,
};

function domainHex(domain: string): number {
  return DOMAIN_HEX[domain] ?? 0x706070;
}

/* ── Ring layout constants ── */

const RING_TILTS = [0.15, 0.55, 0.95, 1.3, 1.7]; // radians, one per domain
const RING_SPEEDS = [0.0008, 0.0006, 0.001, 0.0007, 0.0009];
const RING_RADII = [3.0, 3.6, 2.4, 4.0, 2.8];

/* ── Halo sprite texture ── */

function createHaloTexture(): THREE.Texture {
  const size = 64;
  const canvas = document.createElement('canvas');
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext('2d')!;
  const gradient = ctx.createRadialGradient(
    size / 2, size / 2, 0,
    size / 2, size / 2, size / 2,
  );
  gradient.addColorStop(0, 'rgba(255,255,255,0.4)');
  gradient.addColorStop(0.4, 'rgba(255,255,255,0.1)');
  gradient.addColorStop(1, 'rgba(255,255,255,0)');
  ctx.fillStyle = gradient;
  ctx.fillRect(0, 0, size, size);
  const tex = new THREE.CanvasTexture(canvas);
  return tex;
}

/* ── Build scene objects ── */

interface NodeObj {
  group: THREE.Group;
  domain: string;
  ringIndex: number;
  angle: number;
  id: string;
}

function buildSceneObjects(
  scene: THREE.Scene,
  nodes: KnowledgeNode[],
  edges: KnowledgeEdge[],
) {
  // Group nodes by domain
  const domainGroups = new Map<string, KnowledgeNode[]>();
  for (const n of nodes) {
    const arr = domainGroups.get(n.domain) ?? [];
    arr.push(n);
    domainGroups.set(n.domain, arr);
  }

  const domainOrder = ['gate', 'agent', 'knowledge', 'plan', 'config'];
  const haloTex = createHaloTexture();
  const nodeObjs: NodeObj[] = [];
  const idToObj = new Map<string, NodeObj>();

  // Create ring visuals (subtle guide rings)
  let ringIdx = 0;
  for (const domain of domainOrder) {
    const group = domainGroups.get(domain);
    if (!group || group.length === 0) { ringIdx++; continue; }

    const ri = ringIdx % RING_TILTS.length;
    const radius = RING_RADII[ri];
    const tilt = RING_TILTS[ri];

    // Subtle ring line
    const ringPoints: THREE.Vector3[] = [];
    const segments = 80;
    for (let i = 0; i <= segments; i++) {
      const a = (i / segments) * Math.PI * 2;
      ringPoints.push(new THREE.Vector3(
        Math.cos(a) * radius,
        Math.sin(a) * radius * Math.sin(tilt) * 0.3,
        Math.sin(a) * radius * Math.cos(tilt) * 0.6,
      ));
    }
    const ringGeo = new THREE.BufferGeometry().setFromPoints(ringPoints);
    const ringMat = new THREE.LineBasicMaterial({
      color: domainHex(domain),
      transparent: true,
      opacity: 0.08,
    });
    scene.add(new THREE.Line(ringGeo, ringMat));

    // Create nodes on ring
    for (let ni = 0; ni < group.length; ni++) {
      const node = group[ni];
      const angle = (ni / group.length) * Math.PI * 2;
      const size = 0.2 + (node.weight ?? 1) * 0.15;
      const color = domainHex(node.domain);

      const nodeGroup = new THREE.Group();

      // Solid inner octahedron
      const innerGeo = new THREE.OctahedronGeometry(size, 0);
      const innerMat = new THREE.MeshStandardMaterial({
        color,
        transparent: true,
        opacity: 0.4,
        roughness: 0.6,
        metalness: 0.2,
      });
      nodeGroup.add(new THREE.Mesh(innerGeo, innerMat));

      // Wireframe overlay
      const wireGeo = new THREE.OctahedronGeometry(size * 1.05, 0);
      const wireMat = new THREE.MeshStandardMaterial({
        color,
        transparent: true,
        opacity: 0.6,
        wireframe: true,
      });
      nodeGroup.add(new THREE.Mesh(wireGeo, wireMat));

      // Halo sprite
      const spriteMat = new THREE.SpriteMaterial({
        map: haloTex,
        color,
        transparent: true,
        opacity: 0.35,
        blending: THREE.AdditiveBlending,
      });
      const sprite = new THREE.Sprite(spriteMat);
      sprite.scale.set(size * 4, size * 4, 1);
      nodeGroup.add(sprite);

      scene.add(nodeGroup);

      const obj: NodeObj = { group: nodeGroup, domain, ringIndex: ri, angle, id: node.id };
      nodeObjs.push(obj);
      idToObj.set(node.id, obj);
    }

    ringIdx++;
  }

  // Edges
  const edgeLines: THREE.Line[] = [];
  for (const edge of edges) {
    const src = idToObj.get(edge.source);
    const tgt = idToObj.get(edge.target);
    if (!src || !tgt) continue;

    const lineGeo = new THREE.BufferGeometry();
    lineGeo.setAttribute('position', new THREE.Float32BufferAttribute([0, 0, 0, 0, 0, 0], 3));
    const lineMat = new THREE.LineBasicMaterial({
      color: 0xb0a0b0,
      transparent: true,
      opacity: 0.15,
      blending: THREE.AdditiveBlending,
    });
    const line = new THREE.Line(lineGeo, lineMat);
    scene.add(line);
    edgeLines.push(line);
  }

  // Ambient + point light
  scene.add(new THREE.AmbientLight(0xffffff, 0.4));
  const pointLight = new THREE.PointLight(0xffffff, 0.8);
  pointLight.position.set(5, 5, 5);
  scene.add(pointLight);

  return { nodeObjs, edgeLines, edges, idToObj, haloTex };
}

/* ── Animation: position nodes on rings, update edges ── */

function positionNode(obj: NodeObj, time: number): THREE.Vector3 {
  const ri = obj.ringIndex;
  const radius = RING_RADII[ri];
  const tilt = RING_TILTS[ri];
  const speed = RING_SPEEDS[ri];
  const a = obj.angle + time * speed;

  const x = Math.cos(a) * radius;
  const y = Math.sin(a) * radius * Math.sin(tilt) * 0.3;
  const z = Math.sin(a) * radius * Math.cos(tilt) * 0.6;

  obj.group.position.set(x, y, z);
  // Slow self-rotation
  obj.group.rotation.x += 0.003;
  obj.group.rotation.y += 0.005;

  return new THREE.Vector3(x, y, z);
}

/* ── Component ── */

export default function KnowledgeOrbit({ nodes, edges, height = 400 }: KnowledgeOrbitProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const isVisible = useIntersectionGate(containerRef as RefObject<HTMLElement | null>);
  const threeScene = useThreeScene(containerRef, {
    cameraZ: 8,
    cameraFov: 55,
    clearAlpha: 0,
  });

  const sceneDataRef = useRef<ReturnType<typeof buildSceneObjects> | null>(null);
  const rafRef = useRef<number>(0);
  const nodesRef = useRef(nodes);
  const edgesRef = useRef(edges);
  nodesRef.current = nodes;
  edgesRef.current = edges;

  // Build/rebuild scene objects when nodes/edges change
  useEffect(() => {
    if (!threeScene) return;
    const { scene } = threeScene;

    // Clear previous objects
    while (scene.children.length > 0) {
      scene.remove(scene.children[0]);
    }

    if (nodes.length === 0) {
      sceneDataRef.current = null;
      return;
    }

    sceneDataRef.current = buildSceneObjects(scene, nodes, edges);
  }, [threeScene, nodes, edges]);

  // Animation loop
  useEffect(() => {
    if (!threeScene) return;
    const { scene, camera, renderer } = threeScene;

    let time = 0;
    const animate = () => {
      rafRef.current = requestAnimationFrame(animate);

      if (!isVisible) return;

      time++;
      const data = sceneDataRef.current;
      if (!data) {
        renderer.render(scene, camera);
        return;
      }

      // Position all nodes on their orbital rings
      const positions = new Map<string, THREE.Vector3>();
      for (const obj of data.nodeObjs) {
        const pos = positionNode(obj, time);
        positions.set(obj.id, pos);
      }

      // Update edge line positions
      let edgeIdx = 0;
      for (const edge of data.edges) {
        const srcPos = positions.get(edge.source);
        const tgtPos = positions.get(edge.target);
        if (!srcPos || !tgtPos) continue;
        if (edgeIdx >= data.edgeLines.length) break;

        const line = data.edgeLines[edgeIdx];
        const posAttr = line.geometry.getAttribute('position') as THREE.BufferAttribute;
        posAttr.setXYZ(0, srcPos.x, srcPos.y, srcPos.z);
        posAttr.setXYZ(1, tgtPos.x, tgtPos.y, tgtPos.z);
        posAttr.needsUpdate = true;
        edgeIdx++;
      }

      // Slow camera auto-rotation
      camera.position.x = 8 * Math.sin(time * 0.001);
      camera.position.z = 8 * Math.cos(time * 0.001);
      camera.lookAt(0, 0, 0);

      renderer.render(scene, camera);
    };

    rafRef.current = requestAnimationFrame(animate);

    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [threeScene, isVisible]);

  return (
    <div
      ref={containerRef}
      style={{ width: '100%', height }}
    />
  );
}
