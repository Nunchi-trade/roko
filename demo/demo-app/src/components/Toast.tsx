import {
  createContext,
  useContext,
  useCallback,
  useState,
  useRef,
  useEffect,
  type ReactNode,
} from 'react';
import './Toast.css';

/* ── Types ── */

export type ToastType = 'success' | 'error' | 'warning' | 'info';

export interface ToastAction {
  label: string;
  onClick: () => void;
}

export interface ToastOptions {
  type?: ToastType;
  duration?: number;
  action?: ToastAction;
}

interface ToastEntry {
  id: number;
  message: string;
  type: ToastType;
  duration: number;
  action?: ToastAction;
  exiting: boolean;
  createdAt: number;
}

interface ToastContextValue {
  toast: (message: string, opts?: ToastOptions) => void;
  dismiss: (id: number) => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

const MAX_VISIBLE = 3;
const DEFAULT_DURATION = 3000;

const ICONS: Record<ToastType, string> = {
  success: '\u2713',
  error: '\u2717',
  warning: '\u26A0',
  info: '\u2139',
};

/* ── Single toast item ── */

function ToastItem({
  entry,
  onDismiss,
}: {
  entry: ToastEntry;
  onDismiss: (id: number) => void;
}) {
  const [progress, setProgress] = useState(100);
  const [hovered, setHovered] = useState(false);
  const timerRef = useRef<ReturnType<typeof setInterval>>(undefined);
  const remainRef = useRef(entry.duration);
  const lastTickRef = useRef(Date.now());

  // Tick the countdown progress bar, pausing on hover
  useEffect(() => {
    if (entry.duration <= 0) return;

    lastTickRef.current = Date.now();

    timerRef.current = setInterval(() => {
      if (hovered) {
        lastTickRef.current = Date.now();
        return;
      }
      const now = Date.now();
      const delta = now - lastTickRef.current;
      lastTickRef.current = now;
      remainRef.current -= delta;
      const pct = Math.max(0, (remainRef.current / entry.duration) * 100);
      setProgress(pct);
      if (remainRef.current <= 0) {
        clearInterval(timerRef.current);
        onDismiss(entry.id);
      }
    }, 50);

    return () => clearInterval(timerRef.current);
  }, [entry.duration, entry.id, hovered, onDismiss]);

  return (
    <div
      className={`toast-item${entry.exiting ? ' exiting' : ''}`}
      data-type={entry.type}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      role="alert"
      aria-live="polite"
    >
      <span className="toast-icon">{ICONS[entry.type]}</span>

      <div className="toast-body">
        <div className="toast-message">{entry.message}</div>
        {entry.action && (
          <button
            className="toast-action"
            onClick={() => {
              entry.action!.onClick();
              onDismiss(entry.id);
            }}
          >
            {entry.action.label}
          </button>
        )}
      </div>

      <button
        className="toast-close"
        onClick={() => onDismiss(entry.id)}
        aria-label="Dismiss"
      >
        &times;
      </button>

      {entry.duration > 0 && (
        <div className="toast-progress" style={{ width: `${progress}%` }} />
      )}
    </div>
  );
}

/* ── Provider ── */

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastEntry[]>([]);
  const nextId = useRef(0);

  const dismiss = useCallback((id: number) => {
    setToasts((prev) => {
      const idx = prev.findIndex((t) => t.id === id);
      if (idx < 0 || prev[idx].exiting) return prev;
      const next = [...prev];
      next[idx] = { ...next[idx], exiting: true };
      return next;
    });
    // Remove from DOM after exit animation
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 160);
  }, []);

  const toast = useCallback(
    (message: string, opts?: ToastOptions) => {
      const id = nextId.current++;
      const entry: ToastEntry = {
        id,
        message,
        type: opts?.type ?? 'info',
        duration: opts?.duration ?? DEFAULT_DURATION,
        action: opts?.action,
        exiting: false,
        createdAt: Date.now(),
      };

      setToasts((prev) => {
        const next = [...prev, entry];
        // Auto-dismiss oldest if over max
        if (next.filter((t) => !t.exiting).length > MAX_VISIBLE) {
          const oldest = next.find((t) => !t.exiting);
          if (oldest) {
            dismiss(oldest.id);
          }
        }
        return next;
      });
    },
    [dismiss],
  );

  return (
    <ToastContext.Provider value={{ toast, dismiss }}>
      {children}
      <div className="toast-container">
        {toasts.map((entry) => (
          <ToastItem key={entry.id} entry={entry} onDismiss={dismiss} />
        ))}
      </div>
    </ToastContext.Provider>
  );
}

/* ── Hook ── */

export function useToast(): ToastContextValue {
  const ctx = useContext(ToastContext);
  if (!ctx) {
    throw new Error('useToast must be used within a ToastProvider');
  }
  return ctx;
}
