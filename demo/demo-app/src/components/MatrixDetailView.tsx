import type { BenchTask } from '../lib/bench-types';
import type { MatrixCell } from '../hooks/useMatrixBench';
import './MatrixDetailView.css';

interface MatrixDetailViewProps {
  cells: MatrixCell[][];
  selectedModels: string[];
  presetLabels: string[];
  tasks: BenchTask[];
}

function shortModel(id: string): string {
  return id.split('-').slice(0, 2).join('-');
}

/** Side-by-side detail view: columns per lane, rows per task. */
export default function MatrixDetailView({
  cells,
  selectedModels,
  presetLabels,
  tasks,
}: MatrixDetailViewProps) {
  // Flatten to lanes with column headers
  const lanes = cells.flatMap((row, ri) =>
    row.map((cell, ci) => ({
      label: `${shortModel(selectedModels[ri] ?? '')} / ${presetLabels[ci] ?? ''}`,
      cell,
    })),
  );

  if (lanes.length === 0 || tasks.length === 0) {
    return <p className="bench-empty-text">Configure and launch a matrix run to see results.</p>;
  }

  return (
    <div className="matrix-detail">
      <div className="matrix-detail-scroll">
        <table className="matrix-detail-table">
          <thead>
            <tr>
              <th className="matrix-detail-task-col">Task</th>
              {lanes.map((lane, i) => (
                <th key={i} className="matrix-detail-lane-header">
                  <span className="matrix-detail-lane-label">{lane.label}</span>
                  {lane.cell.passRate != null && (
                    <span className={`matrix-detail-lane-rate ${lane.cell.passRate >= 0.5 ? 'pass' : 'fail'}`}>
                      {(lane.cell.passRate * 100).toFixed(0)}%
                    </span>
                  )}
                  {lane.cell.costUsd != null && (
                    <span className="matrix-detail-lane-cost">${lane.cell.costUsd.toFixed(3)}</span>
                  )}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {tasks.map((task, ti) => (
              <tr key={task.id}>
                <td className="matrix-detail-task-name" title={task.name}>
                  {task.name.length > 24 ? `${task.name.slice(0, 22)}..` : task.name}
                </td>
                {lanes.map((lane, li) => {
                  const result = lane.cell.results.find((r) => r.task_id === task.id);
                  if (!result) {
                    const isRunning = lane.cell.status === 'running' && lane.cell.results.length === ti;
                    return (
                      <td key={li} className="matrix-detail-cell">
                        <span className={`matrix-detail-chip ${isRunning ? 'chip-running' : 'chip-pending'}`} />
                      </td>
                    );
                  }
                  return (
                    <td key={li} className="matrix-detail-cell">
                      <span
                        className={`matrix-detail-chip chip-${result.status}`}
                        title={`${result.status} | $${result.cost_usd.toFixed(3)} | ${(result.duration_ms / 1000).toFixed(1)}s`}
                      />
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
