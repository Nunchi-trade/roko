import { useEffect, useState, useCallback, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import './Drawer.css';

interface DrawerProps {
  open: boolean;
  onClose: () => void;
  position?: 'left' | 'right' | 'bottom';
  width?: number | string;
  height?: number | string;
  title?: string;
  children: ReactNode;
  className?: string;
}

export default function Drawer({
  open,
  onClose,
  position = 'right',
  width = 360,
  height = '40vh',
  title,
  children,
  className,
}: DrawerProps) {
  const [visible, setVisible] = useState(false);
  const [closing, setClosing] = useState(false);

  useEffect(() => {
    if (open) {
      setVisible(true);
      setClosing(false);
      document.body.style.overflow = 'hidden';
    } else if (visible) {
      setClosing(true);
      const timer = setTimeout(() => {
        setVisible(false);
        setClosing(false);
        document.body.style.overflow = '';
      }, 150);
      return () => clearTimeout(timer);
    }
  }, [open, visible]);

  useEffect(() => {
    return () => {
      document.body.style.overflow = '';
    };
  }, []);

  // Escape key
  useEffect(() => {
    if (!visible || closing) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onClose();
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [visible, closing, onClose]);

  const handleBackdropClick = useCallback(
    (e: React.MouseEvent) => {
      if (e.target === e.currentTarget) {
        onClose();
      }
    },
    [onClose],
  );

  if (!visible) return null;

  const isHorizontal = position === 'left' || position === 'right';
  const panelStyle: React.CSSProperties = isHorizontal
    ? { width: typeof width === 'number' ? `${width}px` : width }
    : { height: typeof height === 'number' ? `${height}px` : height };

  const backdropCls = [
    'drawer-backdrop',
    !closing && 'drawer-backdrop--open',
    closing && 'drawer-backdrop--closing',
  ]
    .filter(Boolean)
    .join(' ');

  const panelCls = [
    'drawer-panel',
    `drawer-panel--${position}`,
    className,
  ]
    .filter(Boolean)
    .join(' ');

  return createPortal(
    <div className={backdropCls} onClick={handleBackdropClick} role="presentation">
      <div className={panelCls} style={panelStyle} role="dialog" aria-label={title}>
        {title && (
          <div className="drawer-header">
            <h2 className="drawer-title">{title}</h2>
            <button className="drawer-close" onClick={onClose} aria-label="Close">&times;</button>
          </div>
        )}
        <div className="drawer-body">{children}</div>
      </div>
    </div>,
    document.body,
  );
}
