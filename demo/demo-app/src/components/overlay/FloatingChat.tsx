import { useState, useRef, useEffect, useCallback } from 'react';
import './FloatingChat.css';

interface ChatMessage {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  timestamp: number;
}

interface FloatingChatProps {
  messages: ChatMessage[];
  onSend: (message: string) => void;
  agentName?: string;
  position?: { x: number; y: number };
  minimized?: boolean;
  onMinimize?: () => void;
  streaming?: boolean;
  className?: string;
}

function formatTime(ts: number): string {
  const d = new Date(ts);
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

export default function FloatingChat({
  messages,
  onSend,
  agentName = 'Agent',
  position,
  minimized = false,
  onMinimize,
  streaming = false,
  className,
}: FloatingChatProps) {
  const [draft, setDraft] = useState('');
  const [pos, setPos] = useState(() => position ?? { x: window.innerWidth - 360, y: window.innerHeight - 440 });
  const [isMinimized, setIsMinimized] = useState(minimized);
  const dragging = useRef(false);
  const dragOffset = useRef({ x: 0, y: 0 });
  const headerRef = useRef<HTMLDivElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  // Sync controlled minimized prop
  useEffect(() => {
    setIsMinimized(minimized);
  }, [minimized]);

  // Auto-scroll to bottom on new messages
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages, streaming]);

  // --- Drag logic using pointer capture ---

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    // Only drag from header area
    if (!(e.target as HTMLElement).closest('.fchat-header')) return;
    // Don't drag if clicking buttons
    if ((e.target as HTMLElement).closest('.fchat-btn')) return;

    dragging.current = true;
    dragOffset.current = { x: e.clientX - pos.x, y: e.clientY - pos.y };
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  }, [pos]);

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    if (!dragging.current) return;
    setPos({
      x: e.clientX - dragOffset.current.x,
      y: e.clientY - dragOffset.current.y,
    });
  }, []);

  const handlePointerUp = useCallback(() => {
    dragging.current = false;
  }, []);

  // --- Send ---

  const handleSend = useCallback(() => {
    const text = draft.trim();
    if (!text) return;
    onSend(text);
    setDraft('');
  }, [draft, onSend]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  const toggleMinimize = useCallback(() => {
    const next = !isMinimized;
    setIsMinimized(next);
    onMinimize?.();
  }, [isMinimized, onMinimize]);

  const rootCls = [
    'fchat',
    isMinimized && 'fchat--minimized',
    className,
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <div
      className={rootCls}
      style={{ transform: `translate(${pos.x}px, ${pos.y}px)`, top: 0, left: 0 }}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
    >
      <div className="fchat-header" ref={headerRef}>
        <span className="fchat-agent-name">{agentName}</span>
        <button
          className="fchat-btn"
          onClick={toggleMinimize}
          aria-label={isMinimized ? 'Expand' : 'Minimize'}
        >
          {isMinimized ? '+' : '\u2013'}
        </button>
      </div>

      <div className="fchat-messages">
        {messages.map((msg) => (
          <div key={msg.id} className={`fchat-msg fchat-msg--${msg.role}`}>
            <div>{msg.content}</div>
            <div className="fchat-msg-time">{formatTime(msg.timestamp)}</div>
          </div>
        ))}
        {streaming && (
          <div className="fchat-typing">
            <span className="fchat-typing-dot" />
            <span className="fchat-typing-dot" />
            <span className="fchat-typing-dot" />
          </div>
        )}
        <div ref={messagesEndRef} />
      </div>

      <div className="fchat-input-bar">
        <input
          className="fchat-input"
          type="text"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Send a message..."
        />
        <button
          className="fchat-send"
          onClick={handleSend}
          disabled={!draft.trim()}
          aria-label="Send"
        >
          &uarr;
        </button>
      </div>
    </div>
  );
}
