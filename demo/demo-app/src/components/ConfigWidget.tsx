import { useState, useMemo } from 'react';
import { useRokoConfig } from '../hooks/useRokoConfig';
import './ConfigWidget.css';

/** Floating config pill / panel — shows active model + provider, allows live changes. */
export default function ConfigWidget() {
  const { defaultModel, defaultBackend, providers, isLive, lastSaved, updateModelConfig } =
    useRokoConfig();
  const [open, setOpen] = useState(false);
  const [selModel, setSelModel] = useState('');
  const [selBackend, setSelBackend] = useState('');
  const [saving, setSaving] = useState(false);

  // All models flattened for the model selector
  const allModels = useMemo(
    () => providers.flatMap((p) => p.models.map((m) => ({ ...m, provider: p.provider }))),
    [providers],
  );

  // Short display label — last segment of slug or name
  const displayModel = defaultModel.split('/').pop()?.replace(/-\d{8}$/, '') ?? defaultModel;
  const displayBackend = defaultBackend || '—';

  // Show "saved" flash for 2s after lastSaved
  const showSaved = lastSaved !== null && Date.now() - lastSaved < 2000;

  const handleOpen = () => {
    setSelModel(defaultModel);
    setSelBackend(defaultBackend);
    setOpen(true);
  };

  const handleApply = async () => {
    if (!selModel) return;
    setSaving(true);
    await updateModelConfig(selModel, selBackend);
    setSaving(false);
    setOpen(false);
  };

  // Derive backend from model selection (find its provider)
  const handleModelChange = (slug: string) => {
    setSelModel(slug);
    const match = allModels.find((m) => m.slug === slug);
    if (match) setSelBackend(match.provider);
  };

  const dirty = selModel !== defaultModel || selBackend !== defaultBackend;

  if (!open) {
    return (
      <button className="cw-pill" onClick={handleOpen} title="Config">
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
        <button className="cw-close" onClick={() => setOpen(false)}>
          ×
        </button>
      </div>
      <div className="cw-body">
        <div className="cw-field">
          <label>Model</label>
          <select value={selModel} onChange={(e) => handleModelChange(e.target.value)} disabled={!isLive}>
            {!allModels.length && <option value="">{defaultModel || '—'}</option>}
            {providers.map((p) => (
              <optgroup key={p.provider} label={p.provider}>
                {p.models.map((m) => (
                  <option key={m.slug} value={m.slug}>
                    {m.name}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
        </div>
        <div className="cw-field">
          <label>Backend</label>
          <span className="cw-backend-display">{selBackend || '—'}</span>
        </div>
        <div className="cw-actions">
          <button className="cw-apply" onClick={handleApply} disabled={!isLive || !dirty || saving}>
            {saving ? 'Saving…' : 'Apply'}
          </button>
          {!isLive && <span className="cw-badge demo">DEMO</span>}
          {showSaved && <span className="cw-saved">Saved</span>}
        </div>
      </div>
    </div>
  );
}
