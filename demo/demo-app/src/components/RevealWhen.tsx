import type { ReactNode } from 'react';
import './RevealWhen.css';

export default function RevealWhen({ visible, children }: { visible: boolean; children: ReactNode }) {
  if (!visible) return null;
  return <div className="reveal-when">{children}</div>;
}
