import { useState, useRef, useCallback, useEffect } from 'react';
import { useDataHub } from '../../app/DataHub';
import { useCanvasSetup } from '../../hooks/useCanvasSetup';
import ChainTab from '../dashboard/ChainTab';
import { WS_BASE } from '../../lib/serve-url';
import { stripAnsi } from '../../lib/strip-ansi';

type DrawerTab = 'history' | 'agents' | 'chain';

export default function IsfrTabDrawer() {
  const [activeTab, setActiveTab] = useState<DrawerTab | null>(null);
  const toggle = (tab: DrawerTab) => setActiveTab(activeTab === tab ? null : tab);

  return (
    <div className="isfr-drawer" data-open={activeTab != null}>
      <div className="isfr-drawer__tabs">
        {(['history', 'agents', 'chain'] as const).map((t) => (
          <button key={t} className={`isfr-drawer__tab ${activeTab === t ? 'isfr-drawer__tab--active' : ''}`}
            onClick={() => toggle(t)}>
            {t.toUpperCase()}
          </button>
        ))}
      </div>
      {activeTab && (
        <div className="isfr-drawer__body">
          {activeTab === 'history' && <HistoryPanel />}
          {activeTab === 'agents' && <AgentsPanel />}
          {activeTab === 'chain' && <ChainTab />}
        </div>
      )}
    </div>
  );
}

/* ── History (canvas chart) ── */
function HistoryPanel() {
  const history = useDataHub((s) => s.isfrHistory);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const draw = useCallback((ctx: CanvasRenderingContext2D, w: number, h: number) => {
    ctx.clearRect(0, 0, w, h);
    if (history.length < 2) {
      ctx.fillStyle = 'rgba(200,200,200,0.3)';
      ctx.font = '12px monospace';
      ctx.textAlign = 'center';
      ctx.fillText('Waiting for history data...', w / 2, h / 2);
      return;
    }
    const pad = { t: 16, b: 24, l: 50, r: 16 };
    const cw = w - pad.l - pad.r;
    const ch = h - pad.t - pad.b;
    const vals = history.map((r) => r.compositeBps);
    const min = Math.min(...vals) * 0.95;
    const max = Math.max(...vals) * 1.05;
    const range = max - min || 1;

    // Grid
    ctx.strokeStyle = 'rgba(120,80,96,0.12)';
    ctx.lineWidth = 1;
    for (let i = 0; i <= 4; i++) {
      const y = pad.t + (i / 4) * ch;
      ctx.beginPath(); ctx.moveTo(pad.l, y); ctx.lineTo(w - pad.r, y); ctx.stroke();
      ctx.fillStyle = 'rgba(200,200,200,0.3)'; ctx.font = '10px monospace'; ctx.textAlign = 'right';
      ctx.fillText(`${Math.round(max - (i / 4) * range)}`, pad.l - 6, y + 4);
    }

    // Line
    ctx.beginPath();
    ctx.strokeStyle = '#cc90a8'; ctx.lineWidth = 2; ctx.lineJoin = 'round';
    for (let i = 0; i < vals.length; i++) {
      const x = pad.l + (i / (vals.length - 1)) * cw;
      const y = pad.t + (1 - (vals[i] - min) / range) * ch;
      i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
    }
    ctx.stroke();

    // Gradient fill
    const lastX = pad.l + cw;
    ctx.lineTo(lastX, pad.t + ch); ctx.lineTo(pad.l, pad.t + ch); ctx.closePath();
    const grad = ctx.createLinearGradient(0, pad.t, 0, pad.t + ch);
    grad.addColorStop(0, 'rgba(204,144,168,0.15)');
    grad.addColorStop(1, 'rgba(204,144,168,0)');
    ctx.fillStyle = grad; ctx.fill();
  }, [history]);

  useCanvasSetup(canvasRef, draw, [history]);

  return (
    <div style={{ height: 280 }}>
      <canvas ref={canvasRef} style={{ width: '100%', height: '100%', display: 'block' }} />
    </div>
  );
}

/* ── Agents (WS log) ── */
function AgentsPanel() {
  const [lines, setLines] = useState<string[]>([]);
  const logRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const ws = new WebSocket(`${WS_BASE}/ws/agents`);
    ws.onmessage = (ev) => {
      const txt = stripAnsi(typeof ev.data === 'string' ? ev.data : '');
      if (txt) setLines((prev) => [...prev.slice(-100), txt]);
    };
    ws.onerror = () => setLines((p) => [...p, '[ws error]']);
    return () => ws.close();
  }, []);

  useEffect(() => { logRef.current && (logRef.current.scrollTop = logRef.current.scrollHeight); }, [lines.length]);

  return (
    <div ref={logRef} style={{ maxHeight: 280, overflowY: 'auto', fontFamily: 'var(--mono)',
      fontSize: 11, lineHeight: 1.5, color: 'var(--text-soft)', padding: '4px 8px' }}>
      {!lines.length && <div style={{ color: 'var(--text-ghost)', fontStyle: 'italic' }}>Connecting to agent stream...</div>}
      {lines.map((l, i) => <div key={i} style={{ padding: '1px 0' }}>{l}</div>)}
    </div>
  );
}
