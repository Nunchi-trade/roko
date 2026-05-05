import { useState, useEffect, useMemo, useCallback, useRef } from 'react';
import { useRokoConfig } from '../hooks/useRokoConfig';
import { useToast } from '../components/Toast';
import {
  flattenProviderModels,
  providerForModelKey,
  resolveModelKey,
} from '../lib/config-models';
import './Settings.css';

/* ── Toggle switch ── */
function Toggle({
  checked,
  onChange,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label className="toggle-wrap">
      <input
        type="checkbox"
        checked={checked}
        onChange={e => onChange(e.target.checked)}
        disabled={disabled}
      />
      <span className="toggle-track" />
    </label>
  );
}

/* ── Collapsible section wrapper ── */
function Section({
  title,
  badge,
  children,
  defaultOpen = true,
}: {
  title: string;
  badge?: number;
  children: React.ReactNode;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="settings-section">
      <button
        className="settings-section-toggle"
        onClick={() => setOpen(o => !o)}
        aria-expanded={open}
        aria-controls={`section-${title.toLowerCase().replace(/\s+/g, '-')}`}
        type="button"
      >
        <h2>
          {title}
          {badge != null && <span className="badge">{badge}</span>}
        </h2>
        <span className={`settings-section-chevron${open ? '' : ' collapsed'}`}>&#9662;</span>
      </button>
      <div
        id={`section-${title.toLowerCase().replace(/\s+/g, '-')}`}
        className={`settings-section-body${open ? '' : ' collapsed'}`}
        role="region"
        aria-label={title}
      >
        {children}
      </div>
    </div>
  );
}

/* ── Save toast ── */
function SaveToast({ visible, onDone }: { visible: boolean; onDone: () => void }) {
  const [exiting, setExiting] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);

  useEffect(() => {
    if (!visible) return;
    setExiting(false);
    timer.current = setTimeout(() => {
      setExiting(true);
      setTimeout(onDone, 300);
    }, 2000);
    return () => clearTimeout(timer.current);
  }, [visible, onDone]);

  if (!visible && !exiting) return null;
  return (
    <div className={`settings-toast${exiting ? ' exiting' : ''}`}>
      <span className="toast-check">&#10003;</span>
      Configuration saved
    </div>
  );
}

/* ══════════════════════════════════════════════════════════════
   Main Settings page
   ══════════════════════════════════════════════════════════════ */

export default function Settings() {
  const { fullConfig, providers, isLive, updateConfig } =
    useRokoConfig();
  const { toast } = useToast();

  // Local editing state
  const [model, setModel] = useState('');
  const [backend, setBackend] = useState('');
  const [bareMode, setBareMode] = useState(true);
  const [effort, setEffort] = useState('medium');
  const [clippyEnabled, setClippyEnabled] = useState(true);
  const [skipTests, setSkipTests] = useState(false);
  const [gateMaxIter, setGateMaxIter] = useState(3);

  const [saving, setSaving] = useState(false);
  const [toastVisible, setToastVisible] = useState(false);
  const [resetShaking, setResetShaking] = useState(false);
  const [btnFlash, setBtnFlash] = useState(false);
  // Track whether user is actively editing (don't overwrite during edits)
  const [dirty, setDirty] = useState(false);

  // Track initial values for reset
  const initialRef = useRef({ model: '', backend: '', bareMode: true, effort: 'medium', clippyEnabled: true, skipTests: false, gateMaxIter: 3 });
  // Track which config snapshot we last synced from
  const lastSyncedRef = useRef('');

  // Sync local state from fullConfig (runs on initial load and after save)
  useEffect(() => {
    if (!fullConfig || Object.keys(fullConfig).length === 0) return;

    // Build a fingerprint to detect actual config changes
    const agent = fullConfig.agent as Record<string, unknown> | undefined;
    const gates = fullConfig.gates as Record<string, unknown> | undefined;
    const fingerprint = JSON.stringify([
      agent?.default_model, agent?.default_backend, agent?.bare_mode, agent?.default_effort,
      gates?.clippy_enabled, gates?.skip_tests, gates?.max_iterations,
    ]);

    // Skip if config hasn't changed or user is actively editing
    if (fingerprint === lastSyncedRef.current || dirty) return;
    lastSyncedRef.current = fingerprint;

    if (agent) {
      if (typeof agent.default_model === 'string') setModel(agent.default_model);
      if (typeof agent.default_backend === 'string') setBackend(agent.default_backend);
      if (typeof agent.bare_mode === 'boolean') setBareMode(agent.bare_mode);
      if (typeof agent.default_effort === 'string') setEffort(agent.default_effort);
    }
    if (gates) {
      if (typeof gates.clippy_enabled === 'boolean') setClippyEnabled(gates.clippy_enabled);
      if (typeof gates.skip_tests === 'boolean') setSkipTests(gates.skip_tests);
      if (typeof gates.max_iterations === 'number') setGateMaxIter(gates.max_iterations);
    }
    initialRef.current = {
      model: (agent?.default_model as string) ?? '',
      backend: (agent?.default_backend as string) ?? '',
      bareMode: (agent?.bare_mode as boolean) ?? true,
      effort: (agent?.default_effort as string) ?? 'medium',
      clippyEnabled: (gates?.clippy_enabled as boolean) ?? true,
      skipTests: (gates?.skip_tests as boolean) ?? false,
      gateMaxIter: (gates?.max_iterations as number) ?? 3,
    };
  }, [fullConfig, dirty]);

  const allModels = useMemo(() => flattenProviderModels(providers), [providers]);

  const handleModelChange = (value: string) => {
    setDirty(true);
    const modelKey = resolveModelKey(allModels, value);
    setModel(modelKey);
    const provider = providerForModelKey(allModels, modelKey);
    if (provider) setBackend(provider);
  };

  const handleSave = async () => {
    setSaving(true);
    setToastVisible(false);
    const ok = await updateConfig({
      agent: {
        default_model: model,
        default_backend: backend,
        bare_mode: bareMode,
        default_effort: effort,
      },
      gates: {
        clippy_enabled: clippyEnabled,
        skip_tests: skipTests,
        max_iterations: gateMaxIter,
      },
    });
    if (ok) {
      setBtnFlash(true);
      setTimeout(() => setBtnFlash(false), 600);
      setToastVisible(true);
      toast('Configuration saved', { type: 'success' });
      initialRef.current = { model, backend, bareMode, effort, clippyEnabled, skipTests, gateMaxIter };
      // Allow re-sync from server response
      setDirty(false);
      lastSyncedRef.current = '';
    } else {
      toast('Failed to save configuration', { type: 'error' });
    }
    setSaving(false);
  };

  const handleReset = useCallback(() => {
    if (resetShaking) {
      // Second click — actually reset
      const i = initialRef.current;
      setModel(i.model);
      setBackend(i.backend);
      setBareMode(i.bareMode);
      setEffort(i.effort);
      setClippyEnabled(i.clippyEnabled);
      setSkipTests(i.skipTests);
      setGateMaxIter(i.gateMaxIter);
      setResetShaking(false);
    } else {
      // First click — warning shake
      setResetShaking(true);
      setTimeout(() => setResetShaking(false), 2000);
    }
  }, [resetShaking]);

  const hideToast = useCallback(() => setToastVisible(false), []);

  return (
    <div className={`settings-page${isLive ? '' : ' settings-offline'}`}>
      <div className="settings-header">
        <span className="settings-title">Settings</span>
        <span className="settings-subtitle">manage providers, models, and defaults</span>
        <div className="settings-status">
          <span className={`dot ${isLive ? '' : 'offline'}`} />
          {isLive ? 'connected' : 'offline'}
        </div>
      </div>

      {!isLive && (
        <div className="settings-offline-banner">
          Server offline — start <code>roko serve</code> to manage settings
        </div>
      )}

      {/* ── Providers ── */}
      <Section title="Providers" badge={providers.length}>
        {providers.length === 0 ? (
          <div className="settings-empty">No providers configured</div>
        ) : (
          <table className="settings-table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Kind</th>
                <th>Models</th>
              </tr>
            </thead>
            <tbody>
              {providers.map(p => (
                <tr key={p.provider}>
                  <td className="mono">{p.provider}</td>
                  <td className="mono">{p.kind}</td>
                  <td>{p.models.length}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Section>

      {/* ── Models ── */}
      <Section title="Models" badge={allModels.length}>
        {allModels.length === 0 ? (
          <div className="settings-empty">No models configured</div>
        ) : (
          <table className="settings-table">
            <thead>
              <tr>
                <th>Key</th>
                <th>Slug</th>
                <th>Provider</th>
              </tr>
            </thead>
            <tbody>
              {allModels.map(m => (
                <tr key={m.key}>
                  <td className="mono">{m.key}</td>
                  <td className="mono">{m.slug}</td>
                  <td className="mono">{m.provider}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Section>

      {/* ── Agent Defaults ── */}
      <Section title="Agent Defaults">
        <div className="settings-field">
          <label>Default Model</label>
          <select
            className="select-animate"
            value={model}
            onChange={e => handleModelChange(e.target.value)}
            disabled={!isLive}
          >
            {!allModels.length && <option value="">{model || '--'}</option>}
            {allModels.map(m => (
              <option key={m.key} value={m.key}>
                {m.slug}
              </option>
            ))}
          </select>
        </div>
        <div className="settings-field">
          <label>Default Backend</label>
          <select
            className="select-animate"
            value={backend}
            onChange={e => { setDirty(true); setBackend(e.target.value); }}
            disabled={!isLive}
          >
            {!providers.length && <option value="">{backend || '--'}</option>}
            {providers.map(p => (
              <option key={p.provider} value={p.provider}>
                {p.provider}
              </option>
            ))}
          </select>
        </div>
        <div className="settings-field">
          <label>Effort</label>
          <select className="select-animate" value={effort} onChange={e => { setDirty(true); setEffort(e.target.value); }} disabled={!isLive}>
            <option value="low">Low</option>
            <option value="medium">Medium</option>
            <option value="high">High</option>
          </select>
        </div>
        <div className="settings-field">
          <label>Bare Mode</label>
          <Toggle checked={bareMode} onChange={v => { setDirty(true); setBareMode(v); }} disabled={!isLive} />
        </div>
      </Section>

      {/* ── Gates ── */}
      <Section title="Gates">
        <div className="settings-field">
          <label>Clippy</label>
          <Toggle checked={clippyEnabled} onChange={v => { setDirty(true); setClippyEnabled(v); }} disabled={!isLive} />
        </div>
        <div className="settings-field">
          <label>Skip Tests</label>
          <Toggle checked={skipTests} onChange={v => { setDirty(true); setSkipTests(v); }} disabled={!isLive} />
        </div>
        <div className="settings-field">
          <label>Max Iterations</label>
          <input
            type="text"
            className="input-narrow input-focus-glow"
            value={gateMaxIter}
            onChange={e => { setDirty(true); setGateMaxIter(Number(e.target.value) || 1); }}
            disabled={!isLive}
          />
        </div>
      </Section>

      {/* ── Actions ── */}
      <div className="settings-actions">
        <button
          className={`primary btn-primary-glow${saving ? ' saving' : ''}${btnFlash ? ' success-flash' : ''}`}
          onClick={handleSave}
          disabled={!isLive || saving}
        >
          {saving ? 'Saving...' : 'Save'}
        </button>
        <button
          className={`reset-btn btn-interactive${resetShaking ? ' shaking' : ''}`}
          onClick={handleReset}
          disabled={!isLive}
        >
          {resetShaking ? 'Click again to reset' : 'Reset'}
        </button>
      </div>

      <SaveToast visible={toastVisible} onDone={hideToast} />
    </div>
  );
}
