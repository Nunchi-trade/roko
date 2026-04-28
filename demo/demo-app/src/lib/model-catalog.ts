export interface ModelOption {
  id: string;
  label: string;
  provider: string;
}

export interface ProviderGroup {
  name: string;
  models: ModelOption[];
}

export const MODEL_CATALOG: ProviderGroup[] = [
  {
    name: 'Anthropic',
    models: [
      { id: 'claude-opus-4', label: 'Claude Opus 4', provider: 'anthropic' },
      { id: 'claude-sonnet-4', label: 'Claude Sonnet 4', provider: 'anthropic' },
      { id: 'claude-haiku-4-5', label: 'Claude Haiku 4.5', provider: 'anthropic' },
    ],
  },
  {
    name: 'OpenAI',
    models: [
      { id: 'gpt-4o', label: 'GPT-4o', provider: 'openai' },
      { id: 'gpt-4o-mini', label: 'GPT-4o Mini', provider: 'openai' },
      { id: 'o1-preview', label: 'o1-preview', provider: 'openai' },
    ],
  },
  {
    name: 'Google',
    models: [
      { id: 'gemini-2.0-flash', label: 'Gemini 2.0 Flash', provider: 'google' },
      { id: 'gemini-pro', label: 'Gemini Pro', provider: 'google' },
    ],
  },
  {
    name: 'Zhipu',
    models: [
      { id: 'glm-4', label: 'GLM-4', provider: 'zhipu' },
    ],
  },
  {
    name: 'Moonshot',
    models: [
      { id: 'moonshot-v1', label: 'Moonshot v1', provider: 'moonshot' },
    ],
  },
  {
    name: 'Ollama',
    models: [
      { id: 'llama3', label: 'Llama 3', provider: 'ollama' },
      { id: 'codellama', label: 'Code Llama', provider: 'ollama' },
      { id: 'mistral', label: 'Mistral', provider: 'ollama' },
    ],
  },
];

export const ALL_MODELS = MODEL_CATALOG.flatMap(g => g.models);
