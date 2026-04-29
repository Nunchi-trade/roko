import { useEffect, useRef } from 'react';
import './AgentOutputStream.css';

export interface AgentOutputStreamProps {
  lines: string[];
  agentId: string | null;
}

/** Terminal-style scrolling viewer for live agent output. */
export default function AgentOutputStream({ lines, agentId }: AgentOutputStreamProps) {
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [lines.length]);

  return (
    <div className="agent-output-stream">
      <div className="agent-output-header">
        <span className="agent-output-title">Agent Output</span>
        {agentId && <span className="agent-output-badge">{agentId}</span>}
      </div>

      {lines.length === 0 ? (
        <div className="agent-output-empty">Waiting for agent output...</div>
      ) : (
        <div className="agent-output-body">
          <pre className="agent-output-pre">{lines.join('\n')}</pre>
          <div ref={bottomRef} />
        </div>
      )}
    </div>
  );
}
