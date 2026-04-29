// --- src/scenes/Build.tsx ---
// T3.14: Build scene — chat interface with model selector, streaming, terminal toggle
import { useState, useCallback, useRef, useEffect } from 'react';
import { SplitView } from '../components/layout/SplitView';
import Pane from '../components/Pane';
import { Badge } from '../components/design/Badge';
import './Build.css';

const MODELS = ['claude-sonnet-4-20250514', 'claude-opus-4-20250514', 'gpt-4o', 'o3'] as const;
const PRESETS = [
  'Explain this codebase',
  'Find and fix bugs',
  'Add comprehensive tests',
  'Refactor for clarity',
  'Generate documentation',
];

interface ChatMessage {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  toolCalls?: Array<{ name: string; input: string; output: string }>;
  timestamp: number;
}

export function Build() {
  const [model, setModel] = useState<string>(MODELS[0]);
  const [input, setInput] = useState('');
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [streaming, setStreaming] = useState(false);
  const [showTerminal, setShowTerminal] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  // Auto-scroll on new messages
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages]);

  // Keyboard: T toggles terminal
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      if (e.code === 'KeyT') { e.preventDefault(); setShowTerminal((s) => !s); }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const handleSend = useCallback(async () => {
    if (!input.trim() || streaming) return;
    const userMsg: ChatMessage = {
      id: crypto.randomUUID(),
      role: 'user',
      content: input.trim(),
      timestamp: Date.now(),
    };
    setMessages((prev) => [...prev, userMsg]);
    setInput('');
    setStreaming(true);

    try {
      const res = await fetch('/api/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ model, message: userMsg.content }),
      });

      if (!res.ok) throw new Error(`Chat failed: ${res.status}`);

      // Streaming response
      const reader = res.body?.getReader();
      const decoder = new TextDecoder();
      let assistantContent = '';
      const assistantId = crypto.randomUUID();

      setMessages((prev) => [...prev, {
        id: assistantId,
        role: 'assistant',
        content: '',
        timestamp: Date.now(),
      }]);

      if (reader) {
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          assistantContent += decoder.decode(value, { stream: true });
          setMessages((prev) =>
            prev.map((m) => m.id === assistantId ? { ...m, content: assistantContent } : m)
          );
        }
      }
    } catch (err) {
      setMessages((prev) => [...prev, {
        id: crypto.randomUUID(),
        role: 'assistant',
        content: `Error: ${err instanceof Error ? err.message : 'Unknown error'}`,
        timestamp: Date.now(),
      }]);
    } finally {
      setStreaming(false);
    }
  }, [input, model, streaming]);

  const chatPanel = (
    <div className="build__chat">
      {/* Model selector chips */}
      <div className="build__model-selector">
        {MODELS.map((m) => (
          <button
            key={m}
            className={`build__model-chip ${model === m ? 'build__model-chip--active' : ''}`}
            onClick={() => setModel(m)}
          >
            {m.split('-').slice(0, 2).join(' ')}
          </button>
        ))}
      </div>

      {/* Message thread */}
      <div className="build__messages">
        {messages.length === 0 && (
          <div className="build__empty">
            <p>Select a model and type a prompt, or choose a preset:</p>
            <div className="build__presets">
              {PRESETS.map((p) => (
                <button key={p} className="build__preset" onClick={() => setInput(p)}>
                  {p}
                </button>
              ))}
            </div>
          </div>
        )}
        {messages.map((msg) => (
          <div key={msg.id} className={`build__msg build__msg--${msg.role}`}>
            <Badge variant={msg.role === 'user' ? 'default' : 'info'}>
              {msg.role}
            </Badge>
            <div className="build__msg-content">
              {msg.content || (streaming && msg.role === 'assistant' ? '...' : '')}
            </div>
            {msg.toolCalls?.map((tc, i) => (
              <details key={i} className="build__tool-call">
                <summary>{tc.name}</summary>
                <pre className="build__tool-input">{tc.input}</pre>
                <pre className="build__tool-output">{tc.output}</pre>
              </details>
            ))}
          </div>
        ))}
        <div ref={messagesEndRef} />
      </div>

      {/* Input bar */}
      <div className="build__input-bar">
        <input
          className="build__input"
          type="text"
          placeholder="Type a prompt..."
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') handleSend(); }}
          disabled={streaming}
        />
        <button
          className="build__send"
          onClick={handleSend}
          disabled={!input.trim() || streaming}
        >
          {streaming ? 'Sending...' : 'Send'}
        </button>
      </div>
    </div>
  );

  if (showTerminal) {
    return (
      <div className="build">
        <SplitView
          left={chatPanel}
          right={
            <Pane title="Terminal" flat>
              <div className="build__terminal-placeholder">
                <p style={{ color: 'var(--text-dim)' }}>Terminal panel (T to toggle)</p>
              </div>
            </Pane>
          }
          defaultSplit={60}
        />
      </div>
    );
  }

  return <div className="build">{chatPanel}</div>;
}
