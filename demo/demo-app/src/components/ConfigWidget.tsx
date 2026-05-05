import { useState, useMemo, useCallback } from 'react';
import { useRokoConfig } from '../hooks/useRokoConfig';
import {
  flattenProviderModels,
  modelLabel,
  providerForModelKey,
  resolveModelKey,
} from '../lib/config-models';
import './ConfigWidget.css';

/* ── Section schema ────────────────────────────────────────── */

type FieldType = 'string' | 'number' | 'boolean' | 'string[]' | 'select';

interface FieldDef {
  key: string;
  label: string;
  type: FieldType;
  options?: string[];            // for select
  readOnly?: boolean;
}

interface SectionDef {
  id: string;
  label: string;
  configKey: string;             // top-level key in fullConfig
  fields: FieldDef[];
  readOnly?: boolean;
  note?: string;                 // e.g. "Needs restart"
}

const SECTIONS: SectionDef[] = [
  {
    id: 'agent', label: 'Agent', configKey: 'agent',
    note: 'Needs restart',
    fields: [
      { key: 'default_model', label: 'Default Model', type: 'string' },
      { key: 'default_backend', label: 'Default Backend', type: 'string' },
      { key: 'default_effort', label: 'Effort', type: 'select', options: ['low', 'medium', 'high'] },
      { key: 'bare_mode', label: 'Bare Mode', type: 'boolean' },
    ],
  },
  {
    id: 'gates', label: 'Gates', configKey: 'gates',
    fields: [
      { key: 'clippy_enabled', label: 'Clippy', type: 'boolean' },
      { key: 'skip_tests', label: 'Skip Tests', type: 'boolean' },
      { key: 'max_iterations', label: 'Max Iterations', type: 'number' },
      { key: 'min_rung', label: 'Min Rung', type: 'number' },
      { key: 'max_rung', label: 'Max Rung', type: 'number' },
    ],
  },
  {
    id: 'routing', label: 'Routing', configKey: 'routing',
    fields: [
      { key: 'default_tier', label: 'Default Tier', type: 'number' },
      { key: 'max_tier', label: 'Max Tier', type: 'number' },
      { key: 'force_backend', label: 'Force Backend', type: 'string' },
    ],
  },
  {
    id: 'budget', label: 'Budget', configKey: 'budget',
    fields: [
      { key: 'max_cost_usd', label: 'Max Cost ($)', type: 'number' },
      { key: 'warn_cost_usd', label: 'Warn Cost ($)', type: 'number' },
      { key: 'max_tokens_per_task', label: 'Max Tokens/Task', type: 'number' },
      { key: 'max_parallel_agents', label: 'Max Parallel Agents', type: 'number' },
    ],
  },
  {
    id: 'pipeline', label: 'Pipeline', configKey: 'pipeline',
    fields: [
      { key: 'mechanical', label: 'Mechanical', type: 'string' },
      { key: 'focused', label: 'Focused', type: 'string' },
      { key: 'integrative', label: 'Integrative', type: 'string' },
      { key: 'architectural', label: 'Architectural', type: 'string' },
    ],
  },
  {
    id: 'learning', label: 'Learning', configKey: 'learning',
    fields: [
      { key: 'log_efficiency_events', label: 'Log Efficiency', type: 'boolean' },
      { key: 'replan_on_gate_failure', label: 'Replan on Failure', type: 'boolean' },
      { key: 'cascade_router', label: 'Cascade Router', type: 'boolean' },
      { key: 'experiments', label: 'Experiments', type: 'boolean' },
    ],
  },
  {
    id: 'conductor', label: 'Conductor', configKey: 'conductor',
    fields: [
      { key: 'max_active_agents', label: 'Max Active', type: 'number' },
      { key: 'max_queue_depth', label: 'Max Queue', type: 'number' },
      { key: 'circuit_breaker_threshold', label: 'Circuit Breaker', type: 'number' },
    ],
  },
  {
    id: 'serve', label: 'Serve', configKey: 'serve',
    note: 'Needs restart',
    fields: [
      { key: 'host', label: 'Host', type: 'string' },
      { key: 'port', label: 'Port', type: 'number' },
      { key: 'cors_origins', label: 'CORS Origins', type: 'string[]' },
    ],
  },
  {
    id: 'prd', label: 'PRD', configKey: 'prd',
    fields: [
      { key: 'auto_plan', label: 'Auto Plan', type: 'boolean' },
      { key: 'auto_research', label: 'Auto Research', type: 'boolean' },
    ],
  },
  {
    id: 'project', label: 'Project', configKey: 'project',
    readOnly: true,
    fields: [
      { key: 'name', label: 'Name', type: 'string', readOnly: true },
      { key: 'description', label: 'Description', type: 'string', readOnly: true },
      { key: 'root_dir', label: 'Root Dir', type: 'string', readOnly: true },
    ],
  },
];

/* ── Helpers ───────────────────────────────────────────────── */

function getNestedValue(obj: Record<string, unknown>, sectionKey: string, fieldKey: string): unknown {
  const section = obj[sectionKey] as Record<string, unknown> | undefined;
  return section?.[fieldKey];
}

function renderFieldValue(value: unknown, type: FieldType): string {
  if (value == null) return '';
  if (type === 'string[]' && Array.isArray(value)) return value.join(', ');
  return String(value);
}

function parseFieldValue(raw: string, type: FieldType): unknown {
  switch (type) {
    case 'number': return raw === '' ? undefined : Number(raw);
    case 'boolean': return raw === 'true';
    case 'string[]': return raw.split(',').map((s) => s.trim()).filter(Boolean);
    default: return raw;
  }
}

/* ── Section editor component ──────────────────────────────── */

function SectionEditor({
  section,
  fullConfig,
  isLive,
  onApply,
}: {
  section: SectionDef;
  fullConfig: Record<string, unknown>;
  isLive: boolean;
  onApply: (sectionKey: string, values: Record<string, unknown>) => Promise<void>;
}) {
  const [edits, setEdits] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);

  const currentValues = useMemo(() => {
    const vals: Record<string, string> = {};
    for (const f of section.fields) {
      vals[f.key] = renderFieldValue(getNestedValue(fullConfig, section.configKey, f.key), f.type);
    }
    return vals;
  }, [fullConfig, section]);

  const isDirty = useMemo(
    () => Object.entries(edits).some(([k, v]) => v !== (currentValues[k] ?? '')),
    [edits, currentValues],
  );

  const handleChange = (key: string, value: string) => {
    setEdits((prev) => ({ ...prev, [key]: value }));
  };

  const handleApply = async () => {
    setSaving(true);
    const parsed: Record<string, unknown> = {};
    for (const f of section.fields) {
      const raw = edits[f.key];
      if (raw !== undefined && raw !== (currentValues[f.key] ?? '')) {
        parsed[f.key] = parseFieldValue(raw, f.type);
      }
    }
    await onApply(section.configKey, parsed);
    setEdits({});
    setSaving(false);
  };

  return (
    <div className="cw-section-fields">
      {section.fields.map((f) => {
        const val = edits[f.key] ?? currentValues[f.key] ?? '';
        const ro = section.readOnly || f.readOnly || !isLive;

        if (f.type === 'boolean') {
          return (
            <label key={f.key} className="cw-field-row cw-field-toggle">
              <span className="cw-field-label">{f.label}</span>
              <input
                type="checkbox"
                checked={edits[f.key] !== undefined ? edits[f.key] === 'true' : val === 'true'}
                onChange={(e) => handleChange(f.key, String(e.target.checked))}
                disabled={ro}
              />
            </label>
          );
        }

        if (f.type === 'select' && f.options) {
          return (
            <label key={f.key} className="cw-field-row">
              <span className="cw-field-label">{f.label}</span>
              <select
                className="cw-field-input"
                value={val}
                onChange={(e) => handleChange(f.key, e.target.value)}
                disabled={ro}
              >
                {!val && <option value="">--</option>}
                {f.options.map((o) => <option key={o} value={o}>{o}</option>)}
              </select>
            </label>
          );
        }

        if (f.type === 'number') {
          return (
            <label key={f.key} className="cw-field-row">
              <span className="cw-field-label">{f.label}</span>
              {ro ? (
                <span className="cw-field-ro">{val || '--'}</span>
              ) : (
                <input
                  type="number"
                  className="cw-field-input"
                  value={val}
                  onChange={(e) => handleChange(f.key, e.target.value)}
                />
              )}
            </label>
          );
        }

        // string or string[]
        return (
          <label key={f.key} className="cw-field-row">
            <span className="cw-field-label">{f.label}</span>
            {ro ? (
              <span className="cw-field-ro">{val || '--'}</span>
            ) : (
              <input
                type="text"
                className="cw-field-input"
                value={val}
                onChange={(e) => handleChange(f.key, e.target.value)}
                placeholder={f.type === 'string[]' ? 'comma-separated' : ''}
              />
            )}
          </label>
        );
      })}
      {!section.readOnly && (
        <div className="cw-section-actions">
          <button
            className="cw-apply"
            onClick={handleApply}
            disabled={!isDirty || !isLive || saving}
          >
            {saving ? 'Saving...' : 'Apply'}
          </button>
        </div>
      )}
    </div>
  );
}

/* ── Model selector (reuses provider groups) ──────────────── */

function ModelSection({
  providers,
  allModels,
  defaultModel,
  defaultBackend,
  isLive,
  onApply,
}: {
  providers: ReturnType<typeof useRokoConfig>['providers'];
  allModels: ReturnType<typeof flattenProviderModels>;
  defaultModel: string;
  defaultBackend: string;
  isLive: boolean;
  onApply: (model: string, backend: string) => Promise<void>;
}) {
  const [selModel, setSelModel] = useState('');
  const [saving, setSaving] = useState(false);

  const handleModelChange = (modelKey: string) => {
    const resolved = resolveModelKey(allModels, modelKey);
    setSelModel(resolved);
  };

  const activeModel = selModel || defaultModel;
  const activeBackend = selModel
    ? (providerForModelKey(allModels, selModel) ?? defaultBackend)
    : defaultBackend;
  const dirty = selModel !== '' && selModel !== defaultModel;

  const handleApply = async () => {
    if (!dirty) return;
    setSaving(true);
    await onApply(activeModel, activeBackend);
    setSelModel('');
    setSaving(false);
  };

  return (
    <div className="cw-section-fields">
      <label className="cw-field-row">
        <span className="cw-field-label">Model</span>
        <select
          className="cw-field-input"
          value={activeModel}
          onChange={(e) => handleModelChange(e.target.value)}
          disabled={!isLive}
        >
          {!allModels.length && <option value="">{defaultModel || '--'}</option>}
          {providers.map((p) => (
            <optgroup key={p.provider} label={p.provider}>
              {p.models.map((m) => (
                <option key={m.key} value={m.key}>{m.slug}</option>
              ))}
            </optgroup>
          ))}
        </select>
      </label>
      <label className="cw-field-row">
        <span className="cw-field-label">Backend</span>
        <span className="cw-field-ro">{activeBackend || '--'}</span>
      </label>
      <div className="cw-section-actions">
        <button className="cw-apply" onClick={handleApply} disabled={!dirty || !isLive || saving}>
          {saving ? 'Saving...' : 'Apply'}
        </button>
      </div>
    </div>
  );
}

/* ── Main widget ───────────────────────────────────────────── */

export default function ConfigWidget() {
  const {
    fullConfig, defaultModel, defaultBackend, providers,
    isLive, lastSaved, updateModelConfig, updateConfig,
  } = useRokoConfig();

  const [open, setOpen] = useState(false);

  const allModels = useMemo(() => flattenProviderModels(providers), [providers]);

  const displayModel = modelLabel(allModels, defaultModel) || '--';
  const displayBackend = defaultBackend || '--';
  const showSaved = lastSaved !== null && Date.now() - lastSaved < 2000;

  const handleSectionApply = useCallback(
    async (sectionKey: string, values: Record<string, unknown>) => {
      await updateConfig({ [sectionKey]: values });
      setOpen(false);
    },
    [updateConfig],
  );

  const handleModelApply = useCallback(
    async (model: string, backend: string) => {
      await updateModelConfig(model, backend);
      setOpen(false);
    },
    [updateModelConfig],
  );

  if (!open) {
    return (
      <button className="cw-pill" onClick={() => setOpen(true)} title="Config">
        <span className={`cw-dot ${isLive ? '' : 'offline'}`} />
        <span>{displayModel}</span>
        <span style={{ color: 'var(--text-dim)' }}>/</span>
        <span>{displayBackend}</span>
      </button>
    );
  }

  return (
    <div className="cw-panel">
      <div className="cw-header">
        <span className="cw-header-label">Config</span>
        <div className="cw-header-right">
          {!isLive && <span className="cw-badge demo">DEMO</span>}
          {showSaved && <span className="cw-saved">Saved</span>}
          <button className="cw-close" onClick={() => setOpen(false)}>
            &times;
          </button>
        </div>
      </div>
      <div className="cw-body">
        {/* Model selector as first section */}
        <details className="cw-section" open>
          <summary className="cw-section-header">
            <span>Agent Model</span>
          </summary>
          <ModelSection
            providers={providers}
            allModels={allModels}
            defaultModel={defaultModel}
            defaultBackend={defaultBackend}
            isLive={isLive}
            onApply={handleModelApply}
          />
        </details>

        {/* Config sections */}
        {SECTIONS.map((section) => (
          <details key={section.id} className="cw-section">
            <summary className="cw-section-header">
              <span>{section.label}</span>
              {section.note && <span className="cw-section-note">{section.note}</span>}
              {section.readOnly && <span className="cw-section-note">read-only</span>}
            </summary>
            <SectionEditor
              section={section}
              fullConfig={fullConfig}
              isLive={isLive}
              onApply={handleSectionApply}
            />
          </details>
        ))}
      </div>
    </div>
  );
}
