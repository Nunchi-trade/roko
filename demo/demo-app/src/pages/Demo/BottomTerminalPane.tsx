import { useCallback, useRef, useEffect } from 'react';
import { useTerminal, type TerminalHandle } from '../../hooks/useTerminal';
import { SpinnerIcon } from '../../components/icons/AnimatedIcons';

interface BottomTerminalPaneProps {
  sessionId: string;
  handleRef: React.RefObject<TerminalHandle | null>;
  workspaceDir: string;
}

export default function BottomTerminalPane({
  sessionId,
  handleRef,
  workspaceDir,
}: BottomTerminalPaneProps) {
  const { attach, status, handle, shellWarning } = useTerminal(sessionId);
  const bodyRef = useRef<HTMLDivElement>(null);
  const cdSent = useRef(false);

  useEffect(() => {
    if (handleRef && 'current' in handleRef) {
      (handleRef as React.MutableRefObject<TerminalHandle | null>).current = handle.current;
    }
  }, [handleRef, handle]);

  // Auto-cd into workspace when connected
  useEffect(() => {
    if (status === 'connected' && workspaceDir && !cdSent.current && handle.current?.ws?.readyState === WebSocket.OPEN) {
      cdSent.current = true;
      handle.current.sendRaw(`cd ${workspaceDir}\r`);
    }
  }, [status, workspaceDir, handle]);

  const bodyCallbackRef = useCallback(
    (node: HTMLDivElement | null) => {
      (bodyRef as React.MutableRefObject<HTMLDivElement | null>).current = node;
      attach(node);
    },
    [attach],
  );

  return (
    <div className="demo-bottom-term-body" ref={bodyCallbackRef}>
      {status === 'connecting' && (
        <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <SpinnerIcon size={16} />
        </div>
      )}
      {shellWarning && (
        <div className="demo-term-shell-warning">{shellWarning}</div>
      )}
    </div>
  );
}
