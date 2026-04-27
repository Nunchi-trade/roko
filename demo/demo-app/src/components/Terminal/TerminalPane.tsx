import { useTerminal } from '../../hooks/useTerminal';
import '@xterm/xterm/css/xterm.css';
import './TerminalPane.css';

interface TerminalPaneProps {
  sessionId: string;
  label?: string;
}

export default function TerminalPane({ sessionId, label }: TerminalPaneProps) {
  const { attach, status } = useTerminal(sessionId);

  return (
    <div className="terminal-pane">
      <div className="pane-header">
        <span className={`pane-dot ${status}`} />
        <span className="pane-label">{label ?? sessionId}</span>
        <span className="pane-status">{status}</span>
      </div>
      <div className="pane-body" ref={attach} />
    </div>
  );
}
