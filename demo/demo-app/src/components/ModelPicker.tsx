import type { BenchModel } from '../lib/bench-types';

interface ModelPickerProps {
  models: BenchModel[];
  value: string;
  onChange: (modelId: string) => void;
}

export default function ModelPicker({ models, value, onChange }: ModelPickerProps) {
  // Group by provider
  const grouped = new Map<string, BenchModel[]>();
  for (const m of models) {
    const list = grouped.get(m.provider) ?? [];
    list.push(m);
    grouped.set(m.provider, list);
  }

  return (
    <select
      className="config-input"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      style={{ maxWidth: 420 }}
    >
      {[...grouped.entries()].map(([provider, providerModels]) => (
        <optgroup key={provider} label={provider}>
          {providerModels.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name} (${m.cost_per_1k_input}/1k in, ${m.cost_per_1k_output}/1k out)
            </option>
          ))}
        </optgroup>
      ))}
    </select>
  );
}
