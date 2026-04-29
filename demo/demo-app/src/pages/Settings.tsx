import { useState, useEffect, useMemo } from 'react';
import { useRokoConfig } from '../hooks/useRokoConfig';
import {
  flattenProviderModels,
  providerForModelKey,
  resolveModelKey,
} from '../lib/config-models';
import './Settings.css';

export default function Settings() {
  const { fullConfig, defaultModel, defaultBackend, providers, isLive, updateConfig } =
    useRokoConfig();

  // Local editing state — initialized from fullConfig, only pushed on save
  const [model, setModel] = useState('');
  const [backend, setBackend] = useState('');
  const [bareMode, setBareMode] = useState(true);
  const [effort, setEffort] = useState('medium');
  const [clippyEnabled, setClippyEnabled] = useState(true);
  const [skipTests, setSkipTests] = useState(false);
  const [gateMaxIter, setGateMaxIter] = useState(3);

  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [initialized, setInitialized] = useState(false);

  // Sync local state from fullConfig context (once populated)
  useEffect(() => {
    if (initialized || !fullConfig || Object.keys(fullConfig).length === 0) return;

    const agent = fullConfig.agent as Record<string, unknown> | undefined;
    if (agent) {
      if (typeof agent.default_model === 'string') setModel(agent.default_model);
      if (typeof agent.default_backend === 'string') setBackend(agent.default_backend);
      if (typeof agent.bare_mode === 'boolean') setBareMode(agent.bare_mode);
      if (typeof agent.default_effort === 'string') setEffort(agent.default_effort);
    }
    const gates = fullConfig.gates as Record<string, unknown> | undefined;
    if (gates) {
      if (typeof gates.clippy_enabled === 'boolean') setClippyEnabled(gates.clippy_enabled);
      if (typeof gates.skip_tests === 'boolean') setSkipTests(gates.skip_tests);
      if (typeof gates.max_iterations === 'number') setGateMaxIter(gates.max_iterations);
    }
    setInitialized(true);
  }, [fullConfig, initialized]);

  // Fallback: sync model/backend from derived context if fullConfig not yet available
  useEffect(() => {
    if (!initialized) {
      if (defaultModel) setModel(defaultModel);
      if (defaultBackend) setBackend(defaultBackend);
    }
  }, [defaultModel, defaultBackend, initialized]);

  const allModels = useMemo(() => flattenProviderModels(providers), [providers]);

  useEffect(() => {
    if (!model || allModels.length === 0) return;
    const modelKey = resolveModelKey(allModels, model);
    if (modelKey !== model) setModel(modelKey);
    const provider = providerForModelKey(allModels, modelKey);
    if (provider && backend !== provider) setBackend(provider);
  }, [allModels, backend, model]);

  const handleModelChange = (value: string) => {
    const modelKey = resolveModelKey(allModels, value);
    setModel(modelKey);
    const provider = providerForModelKey(allModels, modelKey);
    if (provider) setBackend(provider);
  };

  const handleSave = async () => {
    setSaving(true);
    setSaved(false);
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
      setSaved(true);
      setTimeout(() => setSaved(false), 2500);
    }
    setSaving(false);
  };

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
      <div className="settings-section">
        <h2>
          Providers
          <span className="badge">{providers.length}</span>
        </h2>
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
      </div>

      {/* ── Models ── */}
      <div className="settings-section">
        <h2>
          Models
          <span className="badge">{allModels.length}</span>
        </h2>
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
      </div>

      {/* ── Agent Defaults ── */}
      <div className="settings-section">
        <h2>Agent Defaults</h2>
        <div className="settings-field">
          <label>Default Model</label>
          <select
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
            value={backend}
            onChange={e => setBackend(e.target.value)}
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
          <select value={effort} onChange={e => setEffort(e.target.value)} disabled={!isLive}>
            <option value="low">Low</option>
            <option value="medium">Medium</option>
            <option value="high">High</option>
          </select>
        </div>
        <div className="settings-field">
          <label>Bare Mode</label>
          <input
            type="checkbox"
            checked={bareMode}
            onChange={e => setBareMode(e.target.checked)}
            disabled={!isLive}
          />
        </div>
      </div>

      {/* ── Gates ── */}
      <div className="settings-section">
        <h2>Gates</h2>
        <div className="settings-field">
          <label>Clippy</label>
          <input
            type="checkbox"
            checked={clippyEnabled}
            onChange={e => setClippyEnabled(e.target.checked)}
            disabled={!isLive}
          />
        </div>
        <div className="settings-field">
          <label>Skip Tests</label>
          <input
            type="checkbox"
            checked={skipTests}
            onChange={e => setSkipTests(e.target.checked)}
            disabled={!isLive}
          />
        </div>
        <div className="settings-field">
          <label>Max Iterations</label>
          <input
            type="text"
            value={gateMaxIter}
            onChange={e => setGateMaxIter(Number(e.target.value) || 1)}
            disabled={!isLive}
            style={{ maxWidth: 80 }}
          />
        </div>
      </div>

      {/* ── Save ── */}
      <div className="settings-actions">
        <button className="primary" onClick={handleSave} disabled={!isLive || saving}>
          {saving ? 'Saving...' : 'Save'}
        </button>
        {saved && <span className="settings-saved">Saved</span>}
      </div>
    </div>
  );
}
