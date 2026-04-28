import { useRef, useEffect } from 'react';
import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';
import { useThreeScene } from '../lib/three-shared';

interface Agent {
  id: string;
  name: string;
  model?: string;
  status: string;
  reputation?: number;
  capabilities?: string[];
  stats?: { tasks?: number; cost?: number };
}

interface Props {
  agents: Agent[];
}

const MODEL_COLORS: Record<string, number> = {
  'claude-opus': 0xb97894,
  'claude-sonnet': 0x7873a5,
  'claude-haiku': 0x6a9ea0,
  'gpt-4o': 0xc39b5f,
};
const DEFAULT_AGENT_COLOR = 0x988090;

interface OrbitAgent {
  mesh: THREE.Mesh;
  trail: THREE.Line;
  trailPositions: Float32Array;
  trailHead: number;
  orbitRadius: number;
  orbitSpeed: number;
  orbitOffset: number;
  orbitTilt: number;
  agent: Agent;
}

export default function AgentFleet3D({ agents }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const three = useThreeScene(containerRef, { cameraZ: 10, cameraFov: 50 });
  const orbitsRef = useRef<OrbitAgent[]>([]);
  const coordinatorRef = useRef<THREE.Mesh | null>(null);
  const hoveredRef = useRef<OrbitAgent | null>(null);

  // Build scene objects when agents change
  useEffect(() => {
    if (!three) return;
    const { scene } = three;

    // Clear previous
    while (scene.children.length > 0) scene.remove(scene.children[0]);

    // Central coordinator
    const coordGeo = new THREE.IcosahedronGeometry(0.5, 1);
    const coordMat = new THREE.MeshBasicMaterial({
      color: 0xb97894,
      wireframe: true,
      transparent: true,
      opacity: 0.6,
    });
    const coordinator = new THREE.Mesh(coordGeo, coordMat);
    scene.add(coordinator);
    coordinatorRef.current = coordinator;

    // Coordinator glow
    const glowGeo = new THREE.IcosahedronGeometry(0.7, 1);
    const glowMat = new THREE.MeshBasicMaterial({
      color: 0xb97894,
      wireframe: true,
      transparent: true,
      opacity: 0.15,
    });
    const glowMesh = new THREE.Mesh(glowGeo, glowMat);
    scene.add(glowMesh);

    if (agents.length === 0) {
      // Empty state: just the coordinator rotating
      const emptyId = { value: 0 };
      const loop = () => {
        coordinator.rotation.y += 0.005;
        coordinator.rotation.x += 0.002;
        glowMesh.rotation.y = coordinator.rotation.y;
        glowMesh.rotation.x = coordinator.rotation.x;
        three.renderer.render(scene, three.camera);
        emptyId.value = requestAnimationFrame(loop);
      };
      emptyId.value = requestAnimationFrame(loop);
      return () => cancelAnimationFrame(emptyId.value);
    }

    // Build agent orbits
    const TRAIL_LEN = 30;
    const orbitAgents: OrbitAgent[] = agents.map((agent, i) => {
      const rep = agent.reputation ?? 50;
      const orbitRadius = 5 - (rep / 100) * 3; // range 2-5
      const orbitSpeed = agent.status === 'active' ? 0.5 : 0.15;
      const orbitOffset = (i / agents.length) * Math.PI * 2;
      const orbitTilt = (Math.random() - 0.5) * 0.6;

      const color = MODEL_COLORS[agent.model ?? ''] ?? DEFAULT_AGENT_COLOR;
      const geo = new THREE.DodecahedronGeometry(0.3, 0);
      const mat = new THREE.MeshBasicMaterial({
        color,
        transparent: true,
        opacity: 0.85,
      });
      const mesh = new THREE.Mesh(geo, mat);
      scene.add(mesh);

      // Trail
      const trailPositions = new Float32Array(TRAIL_LEN * 3);
      const trailGeo = new THREE.BufferGeometry();
      trailGeo.setAttribute('position', new THREE.BufferAttribute(trailPositions, 3));
      const trailMat = new THREE.LineBasicMaterial({
        color,
        transparent: true,
        opacity: 0.2,
      });
      const trail = new THREE.Line(trailGeo, trailMat);
      scene.add(trail);

      return {
        mesh,
        trail,
        trailPositions,
        trailHead: 0,
        orbitRadius,
        orbitSpeed,
        orbitOffset,
        orbitTilt,
        agent,
      };
    });
    orbitsRef.current = orbitAgents;
  }, [three, agents]);

  // Animation loop
  useEffect(() => {
    if (!three) return;
    const { scene, camera, renderer } = three;

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.dampingFactor = 0.08;
    controls.enablePan = false;
    controls.minDistance = 4;
    controls.maxDistance = 25;

    const raycaster = new THREE.Raycaster();
    const mouse = new THREE.Vector2(9999, 9999);

    const onMouseMove = (e: MouseEvent) => {
      const rect = renderer.domElement.getBoundingClientRect();
      mouse.x = ((e.clientX - rect.left) / rect.width) * 2 - 1;
      mouse.y = -((e.clientY - rect.top) / rect.height) * 2 + 1;
    };
    renderer.domElement.addEventListener('mousemove', onMouseMove);

    let animId = 0;
    const TRAIL_LEN = 30;

    const loop = () => {
      const t = performance.now() * 0.001;

      // Rotate coordinator
      const coord = coordinatorRef.current;
      if (coord) {
        coord.rotation.y += 0.003;
        coord.rotation.x += 0.001;
      }

      // Update orbit positions
      const orbitAgents = orbitsRef.current;
      for (const oa of orbitAgents) {
        const angle = t * oa.orbitSpeed + oa.orbitOffset;
        const rx = oa.orbitRadius * 1.15; // slight ellipse
        const rz = oa.orbitRadius;
        const x = Math.cos(angle) * rx;
        const z = Math.sin(angle) * rz;
        const y = Math.sin(angle * 0.5) * oa.orbitTilt;
        oa.mesh.position.set(x, y, z);
        oa.mesh.rotation.y += 0.01;
        oa.mesh.rotation.x += 0.005;

        // Update trail
        const head = oa.trailHead % TRAIL_LEN;
        oa.trailPositions[head * 3] = x;
        oa.trailPositions[head * 3 + 1] = y;
        oa.trailPositions[head * 3 + 2] = z;
        oa.trailHead++;
        const posAttr = oa.trail.geometry.getAttribute('position') as THREE.BufferAttribute;
        posAttr.needsUpdate = true;
        oa.trail.geometry.setDrawRange(0, Math.min(oa.trailHead, TRAIL_LEN));
      }

      // Hover raycast
      raycaster.setFromCamera(mouse, camera);
      const meshes = orbitAgents.map((oa) => oa.mesh);
      const hits = raycaster.intersectObjects(meshes);
      const prev = hoveredRef.current;
      if (prev) {
        prev.mesh.scale.set(1, 1, 1);
        (prev.mesh.material as THREE.MeshBasicMaterial).color.set(
          MODEL_COLORS[prev.agent.model ?? ''] ?? DEFAULT_AGENT_COLOR,
        );
        hoveredRef.current = null;
      }
      if (hits.length > 0) {
        const hit = orbitAgents.find((oa) => oa.mesh === hits[0].object);
        if (hit) {
          hit.mesh.scale.set(1.4, 1.4, 1.4);
          (hit.mesh.material as THREE.MeshBasicMaterial).color.set(0xffffff);
          hoveredRef.current = hit;
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
    <div style={{ position: 'relative', width: '100%', height: 'calc(100vh - 280px)' }}>
      <div
        ref={containerRef}
        style={{ width: '100%', height: '100%', background: 'rgba(6,6,8,.3)', borderRadius: 2 }}
      />
      {agents.length === 0 && (
        <div style={{
          position: 'absolute', top: '50%', left: '50%',
          transform: 'translate(-50%, -50%)',
          fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--text-dim, #6a5a68)',
          textAlign: 'center', pointerEvents: 'none',
        }}>
          No agents
        </div>
      )}
    </div>
  );
}
