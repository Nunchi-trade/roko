export interface ModelOption {
  id: string;
  label: string;
  provider: string;
}

export interface ProviderGroup {
  name: string;
  models: ModelOption[];
}

/**
 * Static model catalog — IDs match `[models.*]` keys in roko.toml.
 * These are what `--model <id>` accepts on the CLI.
 *
 * Used as a fallback when the live config from `/api/config` is unavailable.
 */
export const MODEL_CATALOG: ProviderGroup[] = [
  {
    name: 'Anthropic',
    models: [
      { id: 'opus', label: 'Claude Opus 4.6', provider: 'anthropic' },
      { id: 'sonnet', label: 'Claude Sonnet 4.6', provider: 'anthropic' },
      { id: 'haiku', label: 'Claude Haiku 4.5', provider: 'anthropic' },
    ],
  },
  {
    name: 'OpenAI',
    models: [
      { id: 'gpt41', label: 'GPT-4.1', provider: 'openai' },
      { id: 'gpt41-mini', label: 'GPT-4.1 Mini', provider: 'openai' },
      { id: 'gpt41-nano', label: 'GPT-4.1 Nano', provider: 'openai' },
      { id: 'o3', label: 'o3', provider: 'openai' },
      { id: 'o3-mini', label: 'o3 Mini', provider: 'openai' },
      { id: 'o4-mini', label: 'o4 Mini', provider: 'openai' },
      { id: 'codex-mini', label: 'Codex Mini', provider: 'openai' },
    ],
  },
  {
    name: 'Perplexity',
    models: [
      { id: 'sonar', label: 'Sonar Pro', provider: 'perplexity' },
      { id: 'sonar-reasoning', label: 'Sonar Reasoning Pro', provider: 'perplexity' },
    ],
  },
  {
    name: 'Moonshot',
    models: [
      { id: 'kimi-k26', label: 'Kimi K2.6', provider: 'moonshot' },
      { id: 'kimi-k25', label: 'Kimi K2.5', provider: 'moonshot' },
      { id: 'kimi-k2', label: 'Kimi K2', provider: 'moonshot' },
    ],
  },
  {
    name: 'Zhipu',
    models: [
      { id: 'glm51', label: 'GLM-5.1', provider: 'zhipu' },
      { id: 'glm5-turbo', label: 'GLM-5 Turbo', provider: 'zhipu' },
      { id: 'glm45-flash', label: 'GLM-4.5 Flash', provider: 'zhipu' },
      { id: 'glm4', label: 'GLM-4 Plus', provider: 'zhipu' },
    ],
  },
  {
    name: 'Gemini',
    models: [
      { id: 'gemini-flash', label: 'Gemini 2.5 Flash', provider: 'gemini' },
      { id: 'gemini-pro', label: 'Gemini 2.5 Pro', provider: 'gemini' },
    ],
  },
  {
    name: 'Cerebras',
    models: [
      { id: 'cerebras-70b', label: 'Llama 3.3 70B', provider: 'cerebras' },
      { id: 'cerebras-8b', label: 'Llama 3.1 8B', provider: 'cerebras' },
      { id: 'cerebras-scout', label: 'Llama 4 Scout', provider: 'cerebras' },
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
