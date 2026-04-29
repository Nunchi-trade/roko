import { useState, useCallback, useEffect, useRef, useMemo } from 'react';
import { useTerminal } from '../hooks/useTerminal';
import { enterWorkspace, showCmd, getRoko } from '../lib/terminal-session';
import { useRokoConfig } from '../hooks/useRokoConfig';
import { useWorkspace } from '../hooks/useWorkspace';
import { useToast } from '../components/Toast';
import GateBar from '../components/GateBar';
import Pane from '../components/Pane';
import './Builder.css';

type BuildBtnState = 'idle' | 'running' | 'success' | 'error';

interface BuilderModelOption {
  id: string;
  label: string;
  provider: string;
}

interface BuilderProviderGroup {
  name: string;
  models: BuilderModelOption[];
}

const PRESETS = [
  { label: 'calculator', prompt: 'Build a CLI calculator in Rust' },
  { label: 'REST API', prompt: 'Create a REST API with health check' },
  { label: 'md-html', prompt: 'Write a markdown to HTML converter' },
  { label: 'dedup', prompt: 'Build a file deduplication tool' },
  { label: 'commitgen', prompt: 'Create a git commit message generator' },
  { label: 'web scraper', prompt: 'Build an async web scraper with rate limiting' },
  { label: 'test harness', prompt: 'Create a test harness with fixtures and assertions' },
  { label: 'config parser', prompt: 'Build a TOML/YAML config parser with validation' },
  { label: 'log analyzer', prompt: 'Create a structured log analyzer with filters' },
  { label: 'task queue', prompt: 'Build an async task queue with retries' },
  { label: 'HTTP client', prompt: 'Create an HTTP client with connection pooling' },
  { label: 'JSON validator', prompt: 'Build a JSON schema validator' },
  { label: 'path finder', prompt: 'Create a shortest-path finder with A* algorithm' },
  { label: 'state machine', prompt: 'Build a typed state machine with transitions' },
  { label: 'rate limiter', prompt: 'Create a token bucket rate limiter' },
];

interface FileEntry { name: string; isNew: boolean }

export default function Builder() {
  const [prompt, setPrompt] = useState('');
  const [running, setRunning] = useState(false);
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [gates, setGates] = useState<{ name: string; status: 'pass' | 'fail' | 'pending' | 'skip' }[]>([
    { name: 'compile', status: 'pending' },
    { name: 'test', status: 'pending' },
    { name: 'clippy', status: 'pending' },
    { name: 'diff', status: 'pending' },
  ]);
  const [statusText, setStatusText] = useState('idle');
  const [showModelDropdown, setShowModelDropdown] = useState(false);
  const [btnState, setBtnState] = useState<BuildBtnState>('idle');
  const [terminalFlash, setTerminalFlash] = useState<'' | 'event-flash' | 'event-flash-pass' | 'event-flash-fail'>('');
  const [wsLoading, setWsLoading] = useState(true);
  const [wsPath, setWsPath] = useState<string | null>(null);
  const terminalFlashTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

  // Use only live config for model list.
  const { providers: liveProviders, isLive, defaultModel } = useRokoConfig();
  const { ensureWorkspace } = useWorkspace();
  const { toast } = useToast();

  const { liveModelCatalog, liveAllModels } = useMemo(() => {
    const catalog: BuilderProviderGroup[] = liveProviders.map(p => ({
      name: p.provider,
      models: p.models.map(m => ({
        id: m.name,       // config key — what --model accepts
        label: m.slug,    // API slug as human-readable label
        provider: p.provider,
      })),
    }));
    const all = catalog.flatMap(g => g.models);
    return { liveModelCatalog: catalog, liveAllModels: all };
  }, [liveProviders]);

  const [selectedModel, setSelectedModel] = useState('');
  const [autocompleteItems, setAutocompleteItems] = useState<string[]>([]);

  // Sync selected model when live config loads
  useEffect(() => {
    if (selectedModel && liveAllModels.some((model) => model.id === selectedModel)) return;
    const initial = defaultModel || liveAllModels[0]?.id || '';
    setSelectedModel(initial);
  }, [defaultModel, liveAllModels, selectedModel]);
  const [showAutocomplete, setShowAutocomplete] = useState(false);
  const [autocompleteIdx, setAutocompleteIdx] = useState(-1);
  const inputRef = useRef<HTMLInputElement>(null);
  const modelRef = useRef<HTMLDivElement>(null);
  const workspaceRef = useRef<string | null>(null);
  const setupDoneRef = useRef(false);

  const { attach, status, handle } = useTerminal('builder-pty');

  // Flash the terminal border on key events
  const flashTerminal = useCallback((type: 'event-flash' | 'event-flash-pass' | 'event-flash-fail') => {
    clearTimeout(terminalFlashTimer.current);
    setTerminalFlash(type);
    terminalFlashTimer.current = setTimeout(() => setTerminalFlash(''), 500);
  }, []);

  // Setup workspace on mount — create server-side, then cd into it
  useEffect(() => {
    if (setupDoneRef.current) return;
    const h = handle.current;
    if (!h) return;
    setupDoneRef.current = true;
    setWsLoading(true);
    ensureWorkspace('roko-builder').then(ws => {
      workspaceRef.current = ws.path;
      setWsPath(ws.path);
      setWsLoading(false);
      enterWorkspace(h, ws.path);
    });
  }, [handle, status, ensureWorkspace]);

  // Close model dropdown on outside click
  useEffect(() => {
    function handleClick(e: MouseEvent) {
      if (modelRef.current && !modelRef.current.contains(e.target as Node)) {
        setShowModelDropdown(false);
      }
    }
    document.addEventListener('mousedown', handleClick);
    return () => document.removeEventListener('mousedown', handleClick);
  }, []);

  // Autocomplete from presets
  const updateAutocomplete = useCallback((value: string) => {
    if (!value.trim()) {
      setAutocompleteItems([]);
      setShowAutocomplete(false);
      return;
    }
    const lower = value.toLowerCase();
    const matches = PRESETS
      .map(p => p.prompt)
      .filter(p => p.toLowerCase().includes(lower));
    setAutocompleteItems(matches);
    setShowAutocomplete(matches.length > 0);
    setAutocompleteIdx(-1);
  }, []);

  const submitTask = useCallback(async (text: string) => {
    const h = handle.current;
    if (running || !text.trim() || !h || !selectedModel) return;
    setRunning(true);
    setBtnState('running');
    setStatusText('building...');
    setShowAutocomplete(false);
    setFiles([]);
    setGates(g => g.map(gate => ({ ...gate, status: 'pending' as const })));

    const escaped = text.trim().replace(/["\\`$]/g, '\\$&');
    const cmd = `${getRoko()} run "${escaped}" --model ${selectedModel}`;

    let hadError = false;

    await showCmd(h, cmd, {
      timeout: 120000,
      onGate: (name, gateStatus) => {
        setGates(prev => prev.map(g =>
          g.name === name ? { ...g, status: gateStatus } : g
        ));
        // Flash terminal border on gate results
        if (gateStatus === 'pass') flashTerminal('event-flash-pass');
        else if (gateStatus === 'fail') { flashTerminal('event-flash-fail'); hadError = true; }
        else flashTerminal('event-flash');
      },
      onCost: (cost) => {
        setStatusText(prev => prev.includes('$') ? prev : `${prev} | ${cost}`);
      },
      onTokens: (tokens) => {
        setStatusText(prev => prev.includes('tok') ? prev : `${prev} | ${tokens} tok`);
      },
      onLog: (_cmd, desc) => {
        setStatusText(desc);
        flashTerminal('event-flash');
      },
    });

    // Detect files from terminal output
    const output = h.getOutputBuffer();
    const fileMatches = output.match(/(?:created?|wrote|generated?)\s+(\S+\.\w+)/gi);
    if (fileMatches) {
      const detected = fileMatches.map(m => {
        const parts = m.split(/\s+/);
        return { name: parts[parts.length - 1], isNew: true };
      });
      setFiles(detected);
    }

    // Button flash: success or error
    setBtnState(hadError ? 'error' : 'success');
    setStatusText('complete');
    setRunning(false);
    toast(hadError ? 'Build completed with errors' : 'Build complete', {
      type: hadError ? 'error' : 'success',
    });

    // Reset button after flash animation
    setTimeout(() => setBtnState('idle'), 700);
  }, [running, handle, selectedModel, flashTerminal, toast]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    submitTask(prompt);
  };

  const handleInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setPrompt(e.target.value);
    updateAutocomplete(e.target.value);
  };

  const handleInputKeyDown = (e: React.KeyboardEvent) => {
    if (!showAutocomplete) return;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setAutocompleteIdx(i => Math.min(i + 1, autocompleteItems.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setAutocompleteIdx(i => Math.max(i - 1, -1));
    } else if (e.key === 'Enter' && autocompleteIdx >= 0) {
      e.preventDefault();
      setPrompt(autocompleteItems[autocompleteIdx]);
      setShowAutocomplete(false);
    } else if (e.key === 'Escape') {
      setShowAutocomplete(false);
    }
  };

  const selectAutocomplete = (item: string) => {
    setPrompt(item);
    setShowAutocomplete(false);
    inputRef.current?.focus();
  };

  const currentModelLabel = liveAllModels.find(m => m.id === selectedModel)?.label ?? selectedModel;

  return (
    <div className="builder-page">
      <div className="builder-header">
        <span className="builder-title">Builder</span>
        <span className="builder-info">type a request -- roko builds it live</span>

        {/* Workspace badge */}
        <span className={`builder-workspace-badge${wsLoading ? ' loading' : ''}`}>
          {wsLoading ? (
            <span className="ws-spinner" />
          ) : (
            <span className="ws-path">{wsPath ? wsPath.split('/').pop() : 'workspace'}</span>
          )}
        </span>

        {/* Model selector */}
        <div className="builder-model-select" ref={modelRef}>
          <button
            className="model-select-btn"
            onClick={() => setShowModelDropdown(v => !v)}
            disabled={!isLive || liveAllModels.length === 0}
          >
            {currentModelLabel || 'No live models'}
          </button>
          {showModelDropdown && (
            <div className="model-dropdown">
              {liveModelCatalog.map(group => (
                <div key={group.name} className="model-group">
                  <div className="model-group-label">{group.name}</div>
                  {group.models.map(m => (
                    <button
                      key={m.id}
                      className={`model-option${m.id === selectedModel ? ' active' : ''}`}
                      onClick={() => {
                        setSelectedModel(m.id);
                        setShowModelDropdown(false);
                      }}
                    >
                      {m.label}
                    </button>
                  ))}
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="builder-presets">
          {PRESETS.map(p => (
            <button key={p.label} className="preset-btn" onClick={() => submitTask(p.prompt)} disabled={running}>
              {p.label}
            </button>
          ))}
        </div>
      </div>

      <div className="builder-main">
        <div className="builder-sidebar">
          <Pane title="FILES">
            {files.length === 0 ? (
              <div className="file-placeholder">no project yet</div>
            ) : (
              files.map(f => (
                <div key={f.name} className={`file-entry${f.isNew ? ' new' : ''}`}>
                  <span className="file-icon">{f.isNew ? '+' : '\u00B7'}</span>
                  {f.name}
                </div>
              ))
            )}
          </Pane>
        </div>
        <div className="builder-divider" />
        <div className={`builder-terminal${terminalFlash ? ` ${terminalFlash}` : ''}`}>
          <div className="builder-terminal-inner" ref={attach} />
        </div>
      </div>

      <form className={`builder-input${running ? ' running' : ''}`} onSubmit={handleSubmit}>
        <span className="prompt-marker">{'\u25B8'}</span>
        <div className="builder-input-wrap">
          <input
            ref={inputRef}
            value={prompt}
            onChange={handleInputChange}
            onKeyDown={handleInputKeyDown}
            onFocus={() => updateAutocomplete(prompt)}
            onBlur={() => setTimeout(() => setShowAutocomplete(false), 150)}
            placeholder="describe what to build..."
            disabled={running}
          />
          {showAutocomplete && autocompleteItems.length > 0 && (
            <div className="builder-autocomplete">
              {autocompleteItems.map((item, i) => (
                <button
                  key={item}
                  className={`autocomplete-item${i === autocompleteIdx ? ' active' : ''}`}
                  onMouseDown={() => selectAutocomplete(item)}
                >
                  {item}
                </button>
              ))}
            </div>
          )}
        </div>
        {running && <span className="builder-processing-indicator" aria-hidden="true" />}
        <button
          type="submit"
          className={`btn-build${btnState === 'success' ? ' success-flash' : ''}${btnState === 'error' ? ' error-flash' : ''}`}
          disabled={running || !prompt.trim()}
        >
          {running && (
            <span className="btn-build-progress">
              <svg viewBox="0 0 100 30" preserveAspectRatio="none">
                <rect x="1" y="1" width="98" height="28" rx="4" ry="4" />
              </svg>
            </span>
          )}
          <span className="btn-build-inner">
            {btnState === 'running' && <span className="btn-build-spinner" />}
            {btnState === 'success' ? '\u2713 Done' : btnState === 'error' ? '\u2717 Error' : running ? 'Building...' : 'Build'}
          </span>
        </button>
      </form>

      <div className="builder-gate-bar">
        <GateBar gates={gates} />
      </div>

      <div className="builder-status-bar">
        <span>{statusText}</span>
        <span className="builder-status-conn">
          <span className={`conn-dot ${status}`} />
          {status}
        </span>
        <span>{files.length} files</span>
      </div>
    </div>
  );
}
