import { useEffect, useRef, useCallback, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import './Modal.css';

interface ModalProps {
  open: boolean;
  onClose: () => void;
  title?: string;
  size?: 'sm' | 'md' | 'lg';
  children: ReactNode;
  footer?: ReactNode;
  closeOnBackdrop?: boolean;
  closeOnEscape?: boolean;
  className?: string;
}

export default function Modal({
  open,
  onClose,
  title,
  size = 'md',
  children,
  footer,
  closeOnBackdrop = true,
  closeOnEscape = true,
  className,
}: ModalProps) {
  const [visible, setVisible] = useState(false);
  const [closing, setClosing] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  // --- Open / close lifecycle ---

  useEffect(() => {
    if (open) {
      previousFocusRef.current = document.activeElement as HTMLElement;
      setVisible(true);
      setClosing(false);
      document.body.style.overflow = 'hidden';
    } else if (visible) {
      // Begin close animation
      setClosing(true);
      const timer = setTimeout(() => {
        setVisible(false);
        setClosing(false);
        document.body.style.overflow = '';
        previousFocusRef.current?.focus();
      }, 150);
      return () => clearTimeout(timer);
    }
  }, [open, visible]);

  // Cleanup body overflow on unmount
  useEffect(() => {
    return () => {
      document.body.style.overflow = '';
    };
  }, []);

  // --- Focus trap ---

  const getFocusableElements = useCallback((): HTMLElement[] => {
    if (!panelRef.current) return [];
    const selector =
      'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';
    return Array.from(panelRef.current.querySelectorAll<HTMLElement>(selector));
  }, []);

  // Focus first element on open
  useEffect(() => {
    if (visible && !closing) {
      const timer = setTimeout(() => {
        const els = getFocusableElements();
        if (els.length > 0) {
          els[0].focus();
        } else {
          panelRef.current?.focus();
        }
      }, 50);
      return () => clearTimeout(timer);
    }
  }, [visible, closing, getFocusableElements]);

  // Keyboard: Escape + Tab trap
  useEffect(() => {
    if (!visible || closing) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && closeOnEscape) {
        e.stopPropagation();
        onClose();
        return;
      }

      if (e.key === 'Tab') {
        const focusable = getFocusableElements();
        if (focusable.length === 0) {
          e.preventDefault();
          return;
        }

        const first = focusable[0];
        const last = focusable[focusable.length - 1];

        if (e.shiftKey) {
          if (document.activeElement === first) {
            e.preventDefault();
            last.focus();
          }
        } else {
          if (document.activeElement === last) {
            e.preventDefault();
            first.focus();
          }
        }
      }
    };

    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [visible, closing, closeOnEscape, onClose, getFocusableElements]);

  // --- Backdrop click ---

  const handleBackdropClick = useCallback(
    (e: React.MouseEvent) => {
      if (closeOnBackdrop && e.target === e.currentTarget) {
        onClose();
      }
    },
    [closeOnBackdrop, onClose],
  );

  if (!visible) return null;

  const backdropCls = [
    'modal-backdrop',
    !closing && 'modal-backdrop--open',
    closing && 'modal-backdrop--closing',
  ]
    .filter(Boolean)
    .join(' ');

  const panelCls = [
    'modal-panel',
    `modal-panel--${size}`,
    className,
  ]
    .filter(Boolean)
    .join(' ');

  return createPortal(
    <div className={backdropCls} onClick={handleBackdropClick} role="presentation">
      <div
        ref={panelRef}
        className={panelCls}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
      >
        {title && (
          <div className="modal-header">
            <h2 className="modal-title">{title}</h2>
            <button className="modal-close" onClick={onClose} aria-label="Close">&times;</button>
          </div>
        )}
        <div className="modal-body">{children}</div>
        {footer && <div className="modal-footer">{footer}</div>}
      </div>
    </div>,
    document.body,
  );
}
