import {
  useState,
  useRef,
  useCallback,
  useEffect,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';
import './Tooltip.css';

type Placement = 'top' | 'bottom' | 'left' | 'right';
type Variant = 'default' | 'rich' | 'code';

interface TooltipProps {
  content: ReactNode;
  placement?: Placement;
  variant?: Variant;
  delay?: number;
  children: ReactNode;
}

function getPosition(
  triggerRect: DOMRect,
  tooltipRect: DOMRect,
  desired: Placement,
): { placement: Placement; top: number; left: number } {
  const gap = 8;
  const margin = 8; // viewport margin

  const positions: Record<Placement, { top: number; left: number }> = {
    top: {
      top: triggerRect.top - tooltipRect.height - gap,
      left: triggerRect.left + triggerRect.width / 2 - tooltipRect.width / 2,
    },
    bottom: {
      top: triggerRect.bottom + gap,
      left: triggerRect.left + triggerRect.width / 2 - tooltipRect.width / 2,
    },
    left: {
      top: triggerRect.top + triggerRect.height / 2 - tooltipRect.height / 2,
      left: triggerRect.left - tooltipRect.width - gap,
    },
    right: {
      top: triggerRect.top + triggerRect.height / 2 - tooltipRect.height / 2,
      left: triggerRect.right + gap,
    },
  };

  const flip: Record<Placement, Placement> = {
    top: 'bottom',
    bottom: 'top',
    left: 'right',
    right: 'left',
  };

  let placement = desired;
  let pos = positions[placement];

  // Flip if overflowing viewport
  if (
    pos.top < margin ||
    pos.left < margin ||
    pos.top + tooltipRect.height > window.innerHeight - margin ||
    pos.left + tooltipRect.width > window.innerWidth - margin
  ) {
    const flipped = flip[placement];
    const flippedPos = positions[flipped];
    if (
      flippedPos.top >= margin &&
      flippedPos.left >= margin &&
      flippedPos.top + tooltipRect.height <= window.innerHeight - margin &&
      flippedPos.left + tooltipRect.width <= window.innerWidth - margin
    ) {
      placement = flipped;
      pos = flippedPos;
    }
  }

  // Clamp to viewport
  pos.left = Math.max(margin, Math.min(pos.left, window.innerWidth - tooltipRect.width - margin));
  pos.top = Math.max(margin, Math.min(pos.top, window.innerHeight - tooltipRect.height - margin));

  return { placement, top: pos.top, left: pos.left };
}

export default function Tooltip({
  content,
  placement: desiredPlacement = 'top',
  variant = 'default',
  delay = 300,
  children,
}: TooltipProps) {
  const [visible, setVisible] = useState(false);
  const [exiting, setExiting] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number; placement: Placement } | null>(null);
  const triggerRef = useRef<HTMLSpanElement>(null);
  const tooltipRef = useRef<HTMLDivElement>(null);
  const delayTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const exitTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

  const show = useCallback(() => {
    clearTimeout(exitTimer.current);
    clearTimeout(delayTimer.current);
    delayTimer.current = setTimeout(() => setVisible(true), delay);
  }, [delay]);

  const hide = useCallback(() => {
    clearTimeout(delayTimer.current);
    setExiting(true);
    exitTimer.current = setTimeout(() => {
      setVisible(false);
      setExiting(false);
      setPos(null);
    }, 100);
  }, []);

  // Position after render
  useEffect(() => {
    if (!visible || exiting) return;
    const trigger = triggerRef.current;
    const tooltip = tooltipRef.current;
    if (!trigger || !tooltip) return;

    const triggerRect = trigger.getBoundingClientRect();
    const tooltipRect = tooltip.getBoundingClientRect();
    const result = getPosition(triggerRect, tooltipRect, desiredPlacement);
    setPos(result);
  }, [visible, exiting, desiredPlacement]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      clearTimeout(delayTimer.current);
      clearTimeout(exitTimer.current);
    };
  }, []);

  const variantClass = variant === 'rich' ? ' tooltip-rich' : variant === 'code' ? ' tooltip-code' : '';

  return (
    <>
      <span
        ref={triggerRef}
        className="tooltip-trigger"
        onMouseEnter={show}
        onMouseLeave={hide}
        onFocus={show}
        onBlur={hide}
      >
        {children}
      </span>
      {visible &&
        createPortal(
          <div
            ref={tooltipRef}
            className={`tooltip-portal${exiting ? ' exiting' : ''}`}
            data-placement={pos?.placement ?? desiredPlacement}
            style={
              pos
                ? { top: pos.top, left: pos.left }
                : { top: -9999, left: -9999 }
            }
          >
            <div className={`tooltip-bubble${variantClass}`}>{content}</div>
            <div className="tooltip-arrow" />
          </div>,
          document.body,
        )}
    </>
  );
}
