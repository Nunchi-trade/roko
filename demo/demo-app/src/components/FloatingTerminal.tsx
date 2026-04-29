import { useState, useCallback, useRef, useEffect } from 'react';
import { useTerminal } from '../hooks/useTerminal';
import './FloatingTerminal.css';

const MIN_W = 360;
const MIN_H = 200;

// SVG icon helpers (inline to avoid deps)
const IconTerminal = () => (
  <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
    <polyline points="4 6 7 9 4 12" />
    <line x1="9" y1="12" x2="13" y2="12" />
  </svg>
);

const IconMinus = () => (
  <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
    <line x1="4" y1="8" x2="12" y2="8" />
  </svg>
);

const IconMaximize = () => (
  <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
    <rect x="3" y="3" width="10" height="10" rx="1" />
  </svg>
);

const IconRestore = () => (
  <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
    <rect x="5" y="5" width="8" height="8" rx="1" />
    <polyline points="5 10 3 10 3 3 10 3 10 5" />
  </svg>
);

const IconClose = () => (
  <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
    <line x1="4" y1="4" x2="12" y2="12" />
    <line x1="12" y1="4" x2="4" y2="12" />
  </svg>
);

export default function FloatingTerminal({ sessionId, cwd }: { sessionId?: string; cwd?: string }) {
  const [mode, setMode] = useState<'open' | 'minimized' | 'fullscreen' | 'closed'>('minimized');
  const [pos, setPos] = useState({ x: -1, y: -1 });
  const [size, setSize] = useState({ w: 560, h: 320 });
  const dragRef = useRef<{ startX: number; startY: number; startPosX: number; startPosY: number } | null>(null);
  const resizeRef = useRef<{ startX: number; startY: number; startW: number; startH: number; startPosX: number; startPosY: number } | null>(null);
  const cwdSent = useRef<string | null>(null);
  // Only create terminal session once user opens the terminal
  const [activated, setActivated] = useState(false);

  const id = useRef(sessionId ?? `float-${Date.now().toString(36)}`);
  const { attach, status, handle } = useTerminal(activated ? id.current : undefined);

  // Activate terminal on first open
  const open = useCallback(() => {
    setActivated(true);
    setMode('open');
  }, []);

  // cd to workspace when terminal connects and cwd changes
  useEffect(() => {
    if (status === 'connected' && cwd && cwd !== cwdSent.current && handle.current) {
      cwdSent.current = cwd;
      handle.current.execCmd(`cd ${cwd} 2>/dev/null; clear`);
    }
  }, [status, cwd, handle]);

  // Initialize position to bottom-right on first open
  useEffect(() => {
    if (mode === 'open' && pos.x === -1) {
      setPos({
        x: Math.max(16, window.innerWidth - size.w - 16),
        y: Math.max(16, window.innerHeight - size.h - 16),
      });
    }
  }, [mode, pos.x, size.w, size.h]);

  // Refit terminal when mode changes (minimized -> open, fullscreen toggle)
  useEffect(() => {
    if ((mode === 'open' || mode === 'fullscreen') && handle.current?.fit) {
      requestAnimationFrame(() => handle.current?.fit.fit());
    }
  }, [mode, handle]);

  // ── Drag ──
  const onDragStart = useCallback((e: React.MouseEvent) => {
    if (mode === 'fullscreen') return;
    e.preventDefault();
    dragRef.current = { startX: e.clientX, startY: e.clientY, startPosX: pos.x, startPosY: pos.y };

    const onMove = (ev: MouseEvent) => {
      if (!dragRef.current) return;
      const dx = ev.clientX - dragRef.current.startX;
      const dy = ev.clientY - dragRef.current.startY;
      setPos({
        x: Math.max(0, Math.min(window.innerWidth - 100, dragRef.current.startPosX + dx)),
        y: Math.max(0, Math.min(window.innerHeight - 40, dragRef.current.startPosY + dy)),
      });
    };
    const onUp = () => {
      dragRef.current = null;
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
    };
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  }, [mode, pos]);

  // ── Resize (top-left corner) ──
  const onResizeStart = useCallback((e: React.MouseEvent) => {
    if (mode === 'fullscreen') return;
    e.preventDefault();
    e.stopPropagation();
    resizeRef.current = {
      startX: e.clientX,
      startY: e.clientY,
      startW: size.w,
      startH: size.h,
      startPosX: pos.x,
      startPosY: pos.y,
    };

    const onMove = (ev: MouseEvent) => {
      if (!resizeRef.current) return;
      const dx = resizeRef.current.startX - ev.clientX;
      const dy = resizeRef.current.startY - ev.clientY;
      const newW = Math.max(MIN_W, resizeRef.current.startW + dx);
      const newH = Math.max(MIN_H, resizeRef.current.startH + dy);
      setSize({ w: newW, h: newH });
      setPos({
        x: Math.max(0, resizeRef.current.startPosX - (newW - resizeRef.current.startW)),
        y: Math.max(0, resizeRef.current.startPosY - (newH - resizeRef.current.startH)),
      });
    };
    const onUp = () => {
      resizeRef.current = null;
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
    };
    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  }, [mode, size, pos]);

  const toggleFullscreen = useCallback(() => {
    setMode((m) => (m === 'fullscreen' ? 'open' : 'fullscreen'));
  }, []);

  if (mode === 'closed') return null;

  if (mode === 'minimized') {
    return (
      <button className="float-term-minimized" onClick={open}>
        <span className={`float-term-minimized-dot ${status}`} />
        <IconTerminal />
        Terminal
      </button>
    );
  }

  const isFS = mode === 'fullscreen';
  const style = isFS
    ? undefined
    : { left: pos.x === -1 ? undefined : pos.x, top: pos.y === -1 ? undefined : pos.y, width: size.w, height: size.h };

  return (
    <div
      className={`float-term${isFS ? ' float-term-fullscreen' : ''}`}
      style={style}
    >
      {/* Resize handle (top-left) */}
      {!isFS && <div className="float-term-resize" onMouseDown={onResizeStart} />}

      <div className="float-term-titlebar" onMouseDown={onDragStart}>
        <span className={`float-term-titlebar-dot ${status}`} />
        <span className="float-term-titlebar-label">Command Terminal</span>
        <div className="float-term-titlebar-actions">
          <button className="float-term-action" onClick={() => setMode('minimized')} title="Minimize">
            <IconMinus />
          </button>
          <button className="float-term-action" onClick={toggleFullscreen} title={isFS ? 'Restore' : 'Fullscreen'}>
            {isFS ? <IconRestore /> : <IconMaximize />}
          </button>
          <button className="float-term-action" onClick={() => setMode('closed')} title="Close">
            <IconClose />
          </button>
        </div>
      </div>

      <div className="float-term-body" ref={attach} />
    </div>
  );
}
