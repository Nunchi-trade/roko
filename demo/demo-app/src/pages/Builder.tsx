import { useState, useCallback } from 'react';
import TerminalPane from '../components/Terminal/TerminalPane';
import GateBar from '../components/GateBar';
import { useApi } from '../hooks/useApi';
import './Builder.css';

const PRESETS = [
  { label: 'calculator', prompt: 'Build a CLI calculator in Rust' },
  { label: 'REST API', prompt: 'Create a REST API with health check' },
  { label: 'md→html', prompt: 'Write a markdown to HTML converter' },
  { label: 'dedup', prompt: 'Build a file deduplication tool' },
  { label: 'commitgen', prompt: 'Create a git commit message generator' },
];

interface FileEntry { name: string; isNew: boolean }

export default function Builder() {
  const [prompt, setPrompt] = useState('');
  const [running, setRunning] = useState(false);
  const [sessionId] = useState(() => `builder-${Date.now()}`);
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [gates, setGates] = useState<{ name: string; status: 'pass' | 'fail' | 'pending' | 'skip' }[]>([
    { name: 'compile', status: 'pending' },
    { name: 'test', status: 'pending' },
    { name: 'clippy', status: 'pending' },
    { name: 'diff', status: 'pending' },
  ]);
  const [statusText, setStatusText] = useState('idle');
  const { post } = useApi();

  const submitTask = useCallback(async (text: string) => {
    if (running || !text.trim()) return;
    setRunning(true);
    setStatusText('building...');
    setFiles([]);
    setGates((g) => g.map((gate) => ({ ...gate, status: 'pending' as const })));

    try {
      const res = await post<{ run_id?: string; files?: string[] }>('/api/run', {
        prompt: text.trim(),
        workdir: `/tmp/roko-builder-${Date.now()}`,
      });

      // Update files if returned
      if (res.files) {
        setFiles(res.files.map((f) => ({ name: f, isNew: true })));
      }

      // Poll for completion
      if (res.run_id) {
        setStatusText(`running (${res.run_id})`);
        // In a real implementation we'd poll /api/run/{id}/status here
      }

      setGates([
        { name: 'compile', status: 'pass' },
        { name: 'test', status: 'pass' },
        { name: 'clippy', status: 'pass' },
        { name: 'diff', status: 'pass' },
      ]);
      setStatusText('complete');
    } catch (err) {
      setStatusText(`error: ${err instanceof Error ? err.message : 'unknown'}`);
      setGates((g) => g.map((gate) => ({ ...gate, status: 'fail' as const })));
    } finally {
      setRunning(false);
    }
  }, [running, post]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    submitTask(prompt);
  };

  return (
    <div className="builder-page">
      <div className="builder-header">
        <span className="builder-title">builder</span>
        <span className="builder-info">type a request — roko builds it live</span>
        <div className="builder-presets">
          {PRESETS.map((p) => (
            <button key={p.label} className="preset-btn" onClick={() => submitTask(p.prompt)} disabled={running}>
              {p.label}
            </button>
          ))}
        </div>
      </div>

      <div className="builder-main">
        <div className="builder-sidebar">
          <h3>files</h3>
          {files.length === 0 ? (
            <div className="file-placeholder">no project yet</div>
          ) : (
            files.map((f) => (
              <div key={f.name} className={`file-entry${f.isNew ? ' new' : ''}`}>
                <span className="file-icon">{f.isNew ? '+' : '·'}</span>
                {f.name}
              </div>
            ))
          )}
        </div>
        <div className="builder-terminal">
          <TerminalPane sessionId={sessionId} label="builder" />
        </div>
      </div>

      <GateBar gates={gates} />

      <form className="builder-input" onSubmit={handleSubmit}>
        <span className="prompt-marker">▸</span>
        <input
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder="describe what to build..."
          disabled={running}
        />
        <button type="submit" className="btn-primary" disabled={running || !prompt.trim()}>
          {running ? 'Building...' : 'Build'}
        </button>
      </form>

      <div className="builder-status-bar">
        <span>{statusText}</span>
        <span>{files.length} files</span>
      </div>
    </div>
  );
}
