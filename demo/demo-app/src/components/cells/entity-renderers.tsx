import type { ReactNode } from 'react';
import FlatIcon, { type FlatIconName } from '../FlatIcon';

type EntityType =
  | 'agent'
  | 'task'
  | 'episode'
  | 'signal'
  | 'knowledge'
  | 'plan'
  | 'prd'
  | 'gate-result';

type IconTone = 'rose' | 'bone' | 'dream' | 'success' | 'warning' | 'muted';

interface EntityRendererField {
  key: string;
  label: string;
  mono?: boolean;
}

interface EntityRenderer {
  icon: (props: { size?: number }) => ReactNode;
  label: (entity: Record<string, unknown>) => string;
  color: string;
  detailFields: EntityRendererField[];
}

function makeIcon(name: FlatIconName, tone: IconTone) {
  return ({ size = 14 }: { size?: number }) => (
    <FlatIcon name={name} size={size} tone={tone} />
  );
}

function str(v: unknown): string {
  if (v == null) return '';
  return String(v);
}

const ENTITY_RENDERERS: Record<EntityType, EntityRenderer> = {
  agent: {
    icon: makeIcon('agent', 'rose'),
    label: (e) => str(e.name) || 'Agent',
    color: 'var(--rose-bright)',
    detailFields: [
      { key: 'role', label: 'Role' },
      { key: 'model', label: 'Model', mono: true },
      { key: 'status', label: 'Status' },
      { key: 'taskCount', label: 'Tasks' },
      { key: 'cost', label: 'Cost', mono: true },
      { key: 'turns', label: 'Turns' },
      { key: 'tokens', label: 'Tokens', mono: true },
    ],
  },

  task: {
    icon: makeIcon('task', 'bone'),
    label: (e) => str(e.title) || str(e.name) || 'Task',
    color: 'var(--bone-bright)',
    detailFields: [
      { key: 'status', label: 'Status' },
      { key: 'agent', label: 'Agent' },
      { key: 'duration', label: 'Duration', mono: true },
      { key: 'model', label: 'Model', mono: true },
      { key: 'cost', label: 'Cost', mono: true },
    ],
  },

  episode: {
    icon: makeIcon('database', 'dream'),
    label: (e) => str(e.agent) || str(e.id) || 'Episode',
    color: 'var(--dream-bright)',
    detailFields: [
      { key: 'agent', label: 'Agent' },
      { key: 'timestamp', label: 'Time', mono: true },
      { key: 'turnCount', label: 'Turns' },
      { key: 'tokens', label: 'Tokens', mono: true },
    ],
  },

  signal: {
    icon: makeIcon('activity', 'rose'),
    label: (e) => str(e.kind) || str(e.type) || 'Signal',
    color: 'var(--rose-glow)',
    detailFields: [
      { key: 'kind', label: 'Kind' },
      { key: 'hash', label: 'Hash', mono: true },
      { key: 'timestamp', label: 'Time', mono: true },
      { key: 'source', label: 'Source' },
    ],
  },

  knowledge: {
    icon: makeIcon('explorer', 'success'),
    label: (e) => str(e.topic) || str(e.domain) || 'Knowledge',
    color: 'var(--success)',
    detailFields: [
      { key: 'domain', label: 'Domain' },
      { key: 'topic', label: 'Topic' },
      { key: 'tier', label: 'Tier' },
      { key: 'citations', label: 'Citations' },
      { key: 'confidence', label: 'Confidence', mono: true },
    ],
  },

  plan: {
    icon: makeIcon('workflow', 'bone'),
    label: (e) => str(e.slug) || str(e.name) || 'Plan',
    color: 'var(--bone-bright)',
    detailFields: [
      { key: 'status', label: 'Status' },
      { key: 'taskCount', label: 'Tasks' },
      { key: 'progress', label: 'Progress', mono: true },
      { key: 'createdAt', label: 'Created', mono: true },
    ],
  },

  prd: {
    icon: makeIcon('event', 'warning'),
    label: (e) => str(e.slug) || str(e.title) || 'PRD',
    color: 'var(--warning)',
    detailFields: [
      { key: 'status', label: 'Status' },
      { key: 'slug', label: 'Slug', mono: true },
      { key: 'section', label: 'Section' },
      { key: 'createdAt', label: 'Created', mono: true },
    ],
  },

  'gate-result': {
    icon: makeIcon('gate', 'muted'),
    label: (e) => str(e.name) || str(e.gate) || 'Gate',
    color: 'var(--text-soft)',
    detailFields: [
      { key: 'verdict', label: 'Verdict' },
      { key: 'gate', label: 'Gate' },
      { key: 'threshold', label: 'Threshold', mono: true },
      { key: 'actual', label: 'Actual', mono: true },
      { key: 'rung', label: 'Rung' },
    ],
  },
};

function getRenderer(type: EntityType): EntityRenderer {
  return ENTITY_RENDERERS[type];
}

export { ENTITY_RENDERERS, getRenderer };
export type { EntityType, EntityRenderer, EntityRendererField };
