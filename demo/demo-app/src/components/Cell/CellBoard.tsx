import type { ReactNode } from 'react';
import './CellBoard.css';

interface CellBoardColumn {
  id: string;
  title: string;
  children: ReactNode;
}

interface CellBoardProps {
  columns: CellBoardColumn[];
  className?: string;
}

export function CellBoard({ columns, className }: CellBoardProps) {
  const cls = ['cell-board', className].filter(Boolean).join(' ');

  return (
    <div className={cls}>
      {columns.map((col) => (
        <div key={col.id} className="cell-board__column">
          <div className="cell-board__column-header">{col.title}</div>
          <div className="cell-board__column-body">{col.children}</div>
        </div>
      ))}
    </div>
  );
}
