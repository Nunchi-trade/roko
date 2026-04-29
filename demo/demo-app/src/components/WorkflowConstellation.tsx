import { useEffect, useMemo, useRef, useState } from 'react';
import * as THREE from 'three';
import type {
  PipelinePhase,
  PipelinePlan,
  PipelineRouteTier,
  PipelineTaskStatus,
} from '../lib/prd-pipeline-types';

interface WorkflowConstellationProps {
  phase: PipelinePhase;
  plans: PipelinePlan[];
  gateTotal: number;
  gatePassed: number;
}

interface NodeSpec {
  id: string;
  x: number;
  y: number;
  z: number;
  radius: number;
  color: number;
  pulse: number;
}

const STAGE_NODES: Array<{ id: string; phase: PipelinePhase; x: number; y: number }> = [
  { id: 'job', phase: 'idea', x: -3.1, y: -0.2 },
  { id: 'prd', phase: 'draft', x: -1.85, y: 0.58 },
  { id: 'plan', phase: 'planning', x: -0.55, y: -0.02 },
  { id: 'tasks', phase: 'tasks', x: 0.75, y: 0.52 },
  { id: 'verify', phase: 'implementing', x: 1.95, y: -0.18 },
  { id: 'done', phase: 'complete', x: 3.05, y: 0.28 },
];

const PHASE_ORDER: PipelinePhase[] = [
  'idle',
  'setup',
  'idea',
  'draft',
  'published',
  'planning',
  'tasks',
  'implementing',
  'complete',
  'failed',
];

function phaseRank(phase: PipelinePhase): number {
  return PHASE_ORDER.indexOf(phase);
}

function colorForStatus(status: PipelineTaskStatus): number {
  if (status === 'done') return 0x8a9c86;
  if (status === 'active') return 0xd89ab2;
  if (status === 'failed') return 0xd78787;
  if (status === 'blocked') return 0xd8a878;
  return 0x8888a8;
}

function colorForRoute(tier?: PipelineRouteTier): number {
  if (tier === 'T3') return 0xd89ab2;
  if (tier === 'T2') return 0xa4a4c8;
  return 0xd4c89c;
}

function canUseWebgl(): boolean {
  try {
    const canvas = document.createElement('canvas');
    return Boolean(canvas.getContext('webgl2') || canvas.getContext('webgl'));
  } catch {
    return false;
  }
}

export default function WorkflowConstellation({
  phase,
  plans,
  gateTotal,
  gatePassed,
}: WorkflowConstellationProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [webglAvailable, setWebglAvailable] = useState(true);

  const tasks = useMemo(() => plans.flatMap((plan) => plan.tasks), [plans]);
  const signature = useMemo(
    () => [
      phase,
      gateTotal,
      gatePassed,
      ...tasks.map((task) => `${task.id}:${task.status}:${task.routeTier}:${task.modelHint ?? ''}`),
    ].join('|'),
    [gatePassed, gateTotal, phase, tasks],
  );

  useEffect(() => {
    if (!containerRef.current) return;
    if (!canUseWebgl()) {
      setWebglAvailable(false);
      return;
    }

    const el = containerRef.current;
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true, preserveDrawingBuffer: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.setClearColor(0x000000, 0);
    el.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(42, 1, 0.1, 100);
    camera.position.set(0, 0, 6.5);

    const group = new THREE.Group();
    scene.add(group);

    const activeRank = phase === 'idle' || phase === 'setup' ? phaseRank('idea') : phaseRank(phase);
    const nodeSpecs: NodeSpec[] = STAGE_NODES.map((node) => {
      const rank = phaseRank(node.phase);
      const done = rank < activeRank || phase === 'complete';
      const active = rank === activeRank || (phase === 'published' && node.id === 'prd');
      return {
        id: node.id,
        x: node.x,
        y: node.y,
        z: 0,
        radius: active ? 0.14 : 0.088,
        color: active ? 0xd89ab2 : done ? 0x8a9c86 : 0x635668,
        pulse: active ? 1 : 0,
      };
    });

    const maxTaskNodes = Math.min(tasks.length, 24);
    for (let i = 0; i < maxTaskNodes; i += 1) {
      const task = tasks[i];
      const angle = (i / Math.max(maxTaskNodes, 1)) * Math.PI * 2;
      const ring = task.status === 'active' ? 1.1 : task.status === 'done' ? 1.0 : 1.22;
      nodeSpecs.push({
        id: task.id,
        x: 0.85 + Math.cos(angle) * ring,
        y: -0.72 + Math.sin(angle) * 0.58,
        z: (i % 5) * 0.03,
        radius: task.status === 'active' ? 0.084 : 0.056,
        color: colorForStatus(task.status) || colorForRoute(task.routeTier),
        pulse: task.status === 'active' ? 1 : 0,
      });
    }

    const gateCount = Math.min(gateTotal, 18);
    for (let i = 0; i < gateCount; i += 1) {
      const done = i < gatePassed;
      nodeSpecs.push({
        id: `gate-${i}`,
        x: 1.55 + (i % 6) * 0.23,
        y: -1.38 + Math.floor(i / 6) * 0.2,
        z: 0.05,
        radius: 0.038,
        color: done ? 0x8a9c86 : 0xd4c89c,
        pulse: 0,
      });
    }

    const nodeMeshes: THREE.Mesh[] = [];
    const circle = new THREE.CircleGeometry(1, 32);
    for (const spec of nodeSpecs) {
      const material = new THREE.MeshBasicMaterial({
        color: spec.color,
        transparent: true,
        opacity: spec.id.startsWith('gate-') ? 0.78 : 0.96,
        blending: THREE.AdditiveBlending,
        depthWrite: false,
      });
      const mesh = new THREE.Mesh(circle, material);
      mesh.position.set(spec.x, spec.y, spec.z);
      mesh.scale.setScalar(spec.radius);
      mesh.userData.pulse = spec.pulse;
      mesh.userData.base = spec.radius;
      group.add(mesh);
      nodeMeshes.push(mesh);
    }

    const linePositions: number[] = [];
    const lineColors: number[] = [];
    const addLine = (from: NodeSpec, to: NodeSpec, color: THREE.Color) => {
      linePositions.push(from.x, from.y, from.z - 0.02, to.x, to.y, to.z - 0.02);
      lineColors.push(color.r, color.g, color.b, color.r, color.g, color.b);
    };

    for (let i = 0; i < STAGE_NODES.length - 1; i += 1) {
      const from = nodeSpecs[i];
      const to = nodeSpecs[i + 1];
      const color = i < Math.max(activeRank - 2, 0) ? new THREE.Color(0x8a9c86) : new THREE.Color(0x5a4a58);
      addLine(from, to, color);
    }

    const taskAnchor = nodeSpecs.find((node) => node.id === 'tasks') ?? nodeSpecs[3];
    const verifyAnchor = nodeSpecs.find((node) => node.id === 'verify') ?? nodeSpecs[4];
    for (const spec of nodeSpecs.filter((node) => !STAGE_NODES.some((stage) => stage.id === node.id))) {
      addLine(spec.id.startsWith('gate-') ? verifyAnchor : taskAnchor, spec, new THREE.Color(spec.id.startsWith('gate-') ? 0x766f5f : 0x5f5b78));
    }

    const lineGeometry = new THREE.BufferGeometry();
    lineGeometry.setAttribute('position', new THREE.Float32BufferAttribute(linePositions, 3));
    lineGeometry.setAttribute('color', new THREE.Float32BufferAttribute(lineColors, 3));
    const lineMaterial = new THREE.LineBasicMaterial({
      vertexColors: true,
      transparent: true,
      opacity: 0.58,
      blending: THREE.AdditiveBlending,
    });
    const lines = new THREE.LineSegments(lineGeometry, lineMaterial);
    group.add(lines);

    const resize = () => {
      const rect = el.getBoundingClientRect();
      const width = Math.max(rect.width, 1);
      const height = Math.max(rect.height, 1);
      renderer.setSize(width, height);
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
    };
    resize();

    const ro = new ResizeObserver(resize);
    ro.observe(el);

    const pointer = { x: 0, y: 0 };
    const onPointerMove = (event: PointerEvent) => {
      const rect = el.getBoundingClientRect();
      pointer.x = ((event.clientX - rect.left) / Math.max(rect.width, 1) - 0.5) * 2;
      pointer.y = ((event.clientY - rect.top) / Math.max(rect.height, 1) - 0.5) * 2;
    };
    const onPointerLeave = () => {
      pointer.x = 0;
      pointer.y = 0;
    };
    el.addEventListener('pointermove', onPointerMove);
    el.addEventListener('pointerleave', onPointerLeave);

    let raf = 0;
    const startedAt = performance.now();
    const tick = () => {
      const t = (performance.now() - startedAt) / 1000;
      group.rotation.z = Math.sin(t * 0.28) * 0.018;
      group.rotation.x = pointer.y * 0.055;
      group.rotation.y = Math.sin(t * 0.18) * 0.09 + pointer.x * 0.12;
      for (const mesh of nodeMeshes) {
        const pulse = mesh.userData.pulse as number;
        const base = mesh.userData.base as number;
        const scale = base * (1 + pulse * (Math.sin(t * 3.4) * 0.18 + 0.2));
        mesh.scale.setScalar(scale);
      }
      renderer.render(scene, camera);
      raf = requestAnimationFrame(tick);
    };
    tick();

    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      el.removeEventListener('pointermove', onPointerMove);
      el.removeEventListener('pointerleave', onPointerLeave);
      circle.dispose();
      lineGeometry.dispose();
      lineMaterial.dispose();
      for (const mesh of nodeMeshes) {
        const material = mesh.material;
        if (Array.isArray(material)) material.forEach((m) => m.dispose());
        else material.dispose();
      }
      renderer.dispose();
      renderer.domElement.remove();
    };
  }, [gatePassed, gateTotal, phase, signature, tasks]);

  if (!webglAvailable) {
    return (
      <div className="pipeline-constellation-static" aria-hidden="true">
        <span />
        <span />
        <span />
        <span />
        <span />
        <span />
      </div>
    );
  }

  return <div className="pipeline-constellation" ref={containerRef} aria-hidden="true" />;
}
