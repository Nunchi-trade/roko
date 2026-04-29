import type { BenchModel, ConfigPreset } from '../lib/bench-types';
import type { MatrixCell, MatrixCellStatus, MatrixStatus } from '../hooks/useMatrixBench';
import { DEFAULT_PRESETS } from '../hooks/useMatrixBench';
import './MatrixBuilder.css';

interface MatrixBuilderProps {
  models: BenchModel[];
  selectedModels: string[];
  toggleModel: (id: string) => void;
  presets: ConfigPreset[];
  togglePreset: (id: string) => void;
  cells: MatrixCell[][];
  totalLanes: number;
  matrixStatus: MatrixStatus;
  estimatedCostPerLane: number;
  onLaunch: () => void;
  disabled: boolean;
}

function shortModel(id: string): string {
  return id.split('-').slice(0, 2).join('-');
}

function cellStatusClass(status: MatrixCellStatus): string {
  switch (status) {
    case 'idle': return 'matrix-cell--idle';
    case 'running': return 'matrix-cell--running';
    case 'pass': return 'matrix-cell--pass';
    case 'fail': return 'matrix-cell--fail';
    case 'partial': return 'matrix-cell--partial';
    default: return '';
  }
}

export default function MatrixBuilder({
  models,
  selectedModels,
  toggleModel,
  presets,
  togglePreset,
  cells,
  totalLanes,
  matrixStatus,
  estimatedCostPerLane,
  onLaunch,
  disabled,
}: MatrixBuilderProps) {
  const estimatedTotal = totalLanes * estimatedCostPerLane;
  const isRunning = matrixStatus === 'running';

  return (
    <div className="matrix-builder">
      {/* Model selection (Y-axis) */}
      <div className="matrix-model-select">
        <div className="matrix-section-label">Models</div>
        <div className="matrix-model-list">
          {models.map((m) => (
            <label key={m.id} className={`matrix-model-item${selectedModels.includes(m.id) ? ' selected' : ''}`}>
              <input
                type="checkbox"
                checked={selectedModels.includes(m.id)}
                onChange={() => toggleModel(m.id)}
                disabled={isRunning}
              />
              <span className="matrix-model-name">{shortModel(m.id)}</span>
              <span className="matrix-model-provider">{m.provider}</span>
            </label>
          ))}
          {models.length === 0 && (
            <span className="matrix-empty-hint">No models. Start roko serve.</span>
          )}
        </div>
      </div>

      {/* Preset selection (X-axis header) */}
      <div className="matrix-preset-select">
        <div className="matrix-section-label">Presets</div>
        <div className="matrix-preset-chips">
          {DEFAULT_PRESETS.map((p) => {
            const active = presets.some((pr) => pr.id === p.id);
            return (
              <button
                key={p.id}
                className={`matrix-preset-chip${active ? ' active' : ''}`}
                onClick={() => togglePreset(p.id)}
                disabled={isRunning}
                title={p.description}
              >
                {p.label}
              </button>
            );
          })}
        </div>
      </div>

      {/* Matrix grid */}
      {selectedModels.length > 0 && presets.length > 0 && (
        <div className="matrix-grid-wrap">
          <table className="matrix-grid">
            <thead>
              <tr>
                <th className="matrix-grid-corner"></th>
                {presets.map((p) => (
                  <th key={p.id} className="matrix-grid-col-header">{p.label}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {selectedModels.map((modelId, row) => (
                <tr key={modelId}>
                  <td className="matrix-grid-row-header">{shortModel(modelId)}</td>
                  {presets.map((preset, col) => {
                    const cell = cells[row]?.[col];
                    const st = cell?.status ?? 'idle';
                    return (
                      <td key={preset.id} className={`matrix-grid-cell ${cellStatusClass(st)}`}>
                        <div className="matrix-cell-indicator" />
                        {cell?.passRate != null && (
                          <span className="matrix-cell-rate">{(cell.passRate * 100).toFixed(0)}%</span>
                        )}
                        {cell?.costUsd != null && (
                          <span className="matrix-cell-cost">${cell.costUsd.toFixed(3)}</span>
                        )}
                        {st === 'running' && cell?.results.length != null && cell.results.length > 0 && (
                          <span className="matrix-cell-progress">{cell.results.length}</span>
                        )}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Footer: cost estimate + launch */}
      <div className="matrix-footer">
        <div className="matrix-footer-stats">
          <span className="matrix-footer-stat">
            <span className="matrix-footer-label">Lanes</span>
            <span className="matrix-footer-value">{totalLanes}</span>
          </span>
          <span className="matrix-footer-stat">
            <span className="matrix-footer-label">Est. Cost</span>
            <span className="matrix-footer-value">
              {estimatedTotal > 0 ? `$${estimatedTotal.toFixed(2)}` : '-'}
            </span>
          </span>
          {matrixStatus !== 'idle' && (
            <span className="matrix-footer-stat">
              <span className="matrix-footer-label">Status</span>
              <span className={`matrix-footer-value matrix-status--${matrixStatus}`}>
                {matrixStatus.toUpperCase()}
              </span>
            </span>
          )}
        </div>
        <button
          className="btn"
          onClick={onLaunch}
          disabled={disabled || isRunning || totalLanes === 0}
        >
          {isRunning
            ? 'Running...'
            : `Launch ${totalLanes} Lane${totalLanes !== 1 ? 's' : ''}`}
        </button>
      </div>
    </div>
  );
}
