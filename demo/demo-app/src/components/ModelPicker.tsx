import type { BenchModel } from '../lib/bench-types';

interface ModelPickerProps {
  models: BenchModel[];
  value: string;
  onChange: (modelId: string, provider: string) => void;
  estimatedTasks?: number;
}

function formatCtx(n: number): string {
  return n >= 1_000_000 ? `${(n / 1_000_000).toFixed(0)}M` : `${(n / 1_000).toFixed(0)}K`;
}

export default function ModelPicker({ models, value, onChange, estimatedTasks }: ModelPickerProps) {
  // Group by provider
  const grouped = new Map<string, BenchModel[]>();
  for (const m of models) {
    const list = grouped.get(m.provider) ?? [];
    list.push(m);
    grouped.set(m.provider, list);
  }

  // Find cheapest and most capable
  const sorted = [...models].sort((a, b) => a.cost_per_1k_input - b.cost_per_1k_input);
  const cheapestId = sorted[0]?.id;
  const capableSorted = [...models].sort((a, b) => b.cost_per_1k_output - a.cost_per_1k_output);
  const capableId = capableSorted[0]?.id;

  return (
    <div className="model-picker">
      {[...grouped.entries()].map(([provider, providerModels]) => (
        <div key={provider} className="model-provider-group">
          <div className="model-provider-badge">{provider}</div>
          <div className="model-cards">
            {providerModels.map((m) => {
              const isSelected = value === m.id;
              const estCost = estimatedTasks
                ? ((m.cost_per_1k_input * 2 + m.cost_per_1k_output * 3) * estimatedTasks * 0.8).toFixed(3)
                : null;

              return (
                <button
                  key={m.id}
                  className={`model-card${isSelected ? ' selected' : ''}`}
                  onClick={() => onChange(m.id, m.provider)}
                >
                  <div className="model-card-top">
                    <span className="model-card-name">{m.name}</span>
                    {m.id === cheapestId && <span className="model-badge cheapest">Cheapest</span>}
                    {m.id === capableId && m.id !== cheapestId && <span className="model-badge capable">Top</span>}
                  </div>
                  <div className="model-card-pricing">
                    <span className="model-price">${m.cost_per_1k_input}/1K in</span>
                    <span className="model-price">${m.cost_per_1k_output}/1K out</span>
                  </div>
                  <div className="model-card-meta">
                    <span className="model-ctx">{formatCtx(m.context_window)} ctx</span>
                    <span className="model-ctx">{m.max_tokens.toLocaleString()} max</span>
                  </div>
                  {estCost && (
                    <div className="model-card-estimate">~${estCost} est.</div>
                  )}
                </button>
              );
            })}
          </div>
        </div>
      ))}
    </div>
  );
}
