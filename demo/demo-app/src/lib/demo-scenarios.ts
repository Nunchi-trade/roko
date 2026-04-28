export interface PlaybackStep {
  terminal: number;
  command: string;
  description: string;
  delay_before_ms: number;
  type_speed_ms: number;
  wait_after_ms: number;
  /** Simulated output lines rendered after the command. */
  output?: string[];
}

export interface Scenario {
  id: string;
  title: string;
  subtitle: string;
  panes: 1 | 2;
  labels: string[];
  steps: PlaybackStep[];
}

/* ── ANSI helpers ───────────────────────────────── */
const R = '\x1b[0m';           // reset
const DIM = '\x1b[2m';
const BOLD = '\x1b[1m';
const ROSE = '\x1b[38;5;139m'; // muted rose
const BONE = '\x1b[38;5;223m'; // warm cream
const AMBER = '\x1b[38;5;180m'; // amber
const SAGE = '\x1b[38;5;108m'; // muted green
const LILAC = '\x1b[38;5;183m'; // light purple
const CYAN = '\x1b[38;5;116m'; // muted cyan
const WHITE = '\x1b[38;5;255m'; // bright white
const BGROSE = '\x1b[48;5;52m'; // dark rose bg
const BGGREY = '\x1b[48;5;236m'; // dark grey bg

/* ── Helix editor chrome (simulated TUI) ────────── */
function helixChrome(filename: string, lines: string[], lang = 'markdown'): string[] {
  const width = 58;
  const pad = (s: string, w: number) => s + ' '.repeat(Math.max(0, w - s.length));

  // Top bar: mode + file + position
  const topBar = `${BGROSE}${WHITE} NORMAL ${R}${BGGREY} ${BONE}${filename}${R}${BGGREY}${DIM} [${lang}]${R}${BGGREY}${' '.repeat(Math.max(0, width - filename.length - lang.length - 16))}${R}`;

  // Line-numbered content
  const numbered = lines.map((line, i) => {
    const num = `${DIM}${String(i + 1).padStart(3)}${R} ${ROSE}│${R} ${line}`;
    return num;
  });

  // Bottom status bar
  const bottomBar = `${BGGREY}${DIM} ${pad(`${lines.length} sel 1 ln`, width - 2)}${R}`;

  // Tilde lines to fill
  const tildes = Array.from({ length: 3 }, () => `${DIM}  ~${R}`);

  return [topBar, '', ...numbered, ...tildes, '', bottomBar];
}

export const SCENARIOS: Scenario[] = [
  /* ═══════════════════════════════════════════════════════════
     1. GENESIS — New project from scratch
     ═══════════════════════════════════════════════════════════ */
  {
    id: 'genesis',
    title: 'Genesis',
    subtitle: 'From zero to a fleshed-out PRD in 60 seconds.',
    panes: 1,
    labels: ['workspace'],
    steps: [
      {
        terminal: 0,
        command: 'mktemp -d /tmp/roko-demo-XXXX',
        description: 'Create ephemeral workspace',
        delay_before_ms: 400,
        type_speed_ms: 25,
        wait_after_ms: 1200,
        output: [
          `${SAGE}/tmp/roko-demo-a8f2${R}`,
        ],
      },
      {
        terminal: 0,
        command: 'cd /tmp/roko-demo-a8f2 && git init -q',
        description: 'Initialize git repo',
        delay_before_ms: 600,
        type_speed_ms: 22,
        wait_after_ms: 800,
        output: [
          `${DIM}Initialized empty Git repository in /tmp/roko-demo-a8f2/.git/${R}`,
        ],
      },
      {
        terminal: 0,
        command: 'roko init',
        description: 'Bootstrap roko workspace',
        delay_before_ms: 600,
        type_speed_ms: 30,
        wait_after_ms: 1800,
        output: [
          `${SAGE}✓${R} Created ${BONE}.roko/${R} directory`,
          `${SAGE}✓${R} Created ${BONE}roko.toml${R} config`,
          `${SAGE}✓${R} Initialized signal log`,
          `${SAGE}✓${R} Initialized episode log`,
          `${SAGE}✓${R} Detected providers: ${AMBER}anthropic${R}, ${AMBER}perplexity${R}`,
          '',
          `${ROSE}◆${R} Workspace ready at ${BONE}/tmp/roko-demo-a8f2${R}`,
        ],
      },
      {
        terminal: 0,
        command: 'roko prd idea "Build a distributed task scheduler with priority queues, worker pools, and automatic retry with exponential backoff"',
        description: 'Capture the idea',
        delay_before_ms: 800,
        type_speed_ms: 18,
        wait_after_ms: 2000,
        output: [
          `${SAGE}✓${R} Idea captured:`,
          `  ${BONE}Build a distributed task scheduler with priority queues,${R}`,
          `  ${BONE}worker pools, and automatic retry with exponential backoff${R}`,
          `  slug: ${ROSE}distributed-task-scheduler${R}`,
          `  path: ${DIM}.roko/prd/ideas/distributed-task-scheduler.md${R}`,
        ],
      },
      {
        terminal: 0,
        command: 'roko prd draft new "distributed-task-scheduler"',
        description: 'Agent drafts the PRD',
        delay_before_ms: 1000,
        type_speed_ms: 22,
        wait_after_ms: 6000,
        output: [
          `${ROSE}▸ researching${R} distributed-task-scheduler`,
          `  ${DIM}querying knowledge store...${R}`,
          `  ${DIM}searching web for prior art...${R}`,
          `  ${SAGE}✓${R} found ${AMBER}3${R} relevant references`,
          '',
          `${ROSE}▸ composing prompt${R} (9 layers)`,
          `  role: ${BONE}architect${R}  model: ${BONE}claude-sonnet-4${R}`,
          '',
          `${ROSE}▸ agent writing${R}`,
          `  ${DIM}├─${R} problem statement ${SAGE}✓${R}`,
          `  ${DIM}├─${R} requirements (14 items) ${SAGE}✓${R}`,
          `  ${DIM}├─${R} architecture overview ${SAGE}✓${R}`,
          `  ${DIM}├─${R} data model ${SAGE}✓${R}`,
          `  ${DIM}├─${R} API surface ${SAGE}✓${R}`,
          `  ${DIM}├─${R} failure modes ${SAGE}✓${R}`,
          `  ${DIM}├─${R} implementation plan (8 tasks) ${SAGE}✓${R}`,
          `  ${DIM}└─${R} success criteria ${SAGE}✓${R}`,
          '',
          `${SAGE}✓ PRD drafted${R}  ${AMBER}2,847 tokens${R}  cost: ${BONE}$0.034${R}  time: ${AMBER}6.2s${R}`,
          `  path: ${DIM}.roko/prd/drafts/distributed-task-scheduler.md${R}`,
        ],
      },
      {
        terminal: 0,
        command: 'hx .roko/prd/drafts/distributed-task-scheduler.md',
        description: 'Open PRD in Helix editor',
        delay_before_ms: 1200,
        type_speed_ms: 28,
        wait_after_ms: 8000,
        output: helixChrome('distributed-task-scheduler.md', [
          `${LILAC}# Distributed Task Scheduler${R}`,
          '',
          `${DIM}> PRD-0001 · status: draft · author: architect agent${R}`,
          '',
          `${LILAC}## Problem Statement${R}`,
          '',
          `${WHITE}Production workloads need a task scheduler that distributes${R}`,
          `${WHITE}work across heterogeneous worker pools with configurable${R}`,
          `${WHITE}priority queues, automatic retry with exponential backoff,${R}`,
          `${WHITE}and observability into queue depth and worker utilization.${R}`,
          '',
          `${LILAC}## Requirements${R}`,
          '',
          `${CYAN}- ${R}${WHITE}R1: Priority queue with at least 4 priority levels${R}`,
          `${CYAN}- ${R}${WHITE}R2: Worker pool with configurable concurrency limits${R}`,
          `${CYAN}- ${R}${WHITE}R3: Exponential backoff retry (base 2s, max 5min, jitter)${R}`,
          `${CYAN}- ${R}${WHITE}R4: Dead-letter queue after max retries exhausted${R}`,
          `${CYAN}- ${R}${WHITE}R5: Prometheus metrics for queue depth + latency p99${R}`,
          `${CYAN}- ${R}${WHITE}R6: Graceful shutdown with in-flight task draining${R}`,
          `${CYAN}- ${R}${WHITE}R7: Task deduplication via idempotency keys${R}`,
          '',
          `${LILAC}## Architecture${R}`,
          '',
          `${DIM}\`\`\`${R}`,
          `${BONE}  ┌──────────┐    ┌────────────┐    ┌──────────┐${R}`,
          `${BONE}  │ Producer │───▸│ PriorityQ  │───▸│ Workers  │${R}`,
          `${BONE}  └──────────┘    │ (BTreeMap) │    │ (pool:N) │${R}`,
          `${BONE}                  └─────┬──────┘    └────┬─────┘${R}`,
          `${BONE}                        │ fail           │ done${R}`,
          `${BONE}                  ┌─────▾──────┐    ┌────▾─────┐${R}`,
          `${BONE}                  │  Retry Q   │    │  Results │${R}`,
          `${BONE}                  │ (backoff)  │    │ (signal) │${R}`,
          `${BONE}                  └────────────┘    └──────────┘${R}`,
          `${DIM}\`\`\`${R}`,
          '',
          `${LILAC}## Implementation Plan${R}`,
          '',
          `${AMBER}1.${R} ${WHITE}Define Task + Priority types and queue trait${R}`,
          `${AMBER}2.${R} ${WHITE}Implement BTreeMap-backed priority queue${R}`,
          `${AMBER}3.${R} ${WHITE}Build worker pool with tokio::JoinSet${R}`,
          `${AMBER}4.${R} ${WHITE}Add retry logic with backoff + jitter${R}`,
          `${AMBER}5.${R} ${WHITE}Wire dead-letter queue${R}`,
          `${AMBER}6.${R} ${WHITE}Add Prometheus metrics endpoints${R}`,
          `${AMBER}7.${R} ${WHITE}Implement graceful shutdown${R}`,
          `${AMBER}8.${R} ${WHITE}Integration tests with multi-worker scenarios${R}`,
        ]),
      },
      {
        terminal: 0,
        command: 'roko doctor',
        description: 'Verify workspace health',
        delay_before_ms: 1500,
        type_speed_ms: 30,
        wait_after_ms: 2000,
        output: [
          `${SAGE}✓${R} .roko/ directory`,
          `${SAGE}✓${R} roko.toml config`,
          `${SAGE}✓${R} signal log (4 entries)`,
          `${SAGE}✓${R} episode log (1 entry)`,
          `${SAGE}✓${R} PRD store (1 draft)`,
          `${SAGE}✓${R} claude CLI available`,
          '',
          `${SAGE}6/6 checks passed${R} — workspace healthy`,
        ],
      },
    ],
  },

  /* ═══════════════════════════════════════════════════════════
     2. SELF-HOSTING — Watch roko develop itself
     ═══════════════════════════════════════════════════════════ */
  {
    id: 'selfhost',
    title: 'Self-Hosting',
    subtitle: 'Watch roko develop itself — from idea to running code.',
    panes: 1,
    labels: ['roko'],
    steps: [
      {
        terminal: 0,
        command: 'cd $(mktemp -d /tmp/roko-selfhost-XXXX) && roko init -q',
        description: 'Create ephemeral workspace',
        delay_before_ms: 400,
        type_speed_ms: 22,
        wait_after_ms: 1000,
        output: [
          `${SAGE}✓${R} Workspace ready at ${BONE}/tmp/roko-selfhost-c4e1${R}`,
        ],
      },
      {
        terminal: 0,
        command: 'roko status',
        description: 'Check workspace status',
        delay_before_ms: 500,
        type_speed_ms: 30,
        wait_after_ms: 3000,
        output: [
          `${ROSE}┌─ roko status ─────────────────────────────────${R}`,
          `${ROSE}│${R}  workspace    ${BONE}/tmp/roko-selfhost-c4e1${R}`,
          `${ROSE}│${R}  version      ${BONE}0.9.2${R}`,
          `${ROSE}│${R}  signals      ${AMBER}1,247${R}`,
          `${ROSE}│${R}  episodes     ${AMBER}847${R}`,
          `${ROSE}│${R}  agents       ${SAGE}5 registered${R}  ${SAGE}3 active${R}`,
          `${ROSE}│${R}  plans        ${AMBER}23 completed${R}  ${BONE}2 active${R}`,
          `${ROSE}│${R}  gate pass    ${SAGE}93.4%${R}`,
          `${ROSE}│${R}  total cost   ${BONE}$1.42${R}  (cascade-routed)`,
          `${ROSE}└──────────────────────────────────────────────${R}`,
        ],
      },
      {
        terminal: 0,
        command: 'roko prd idea "Add retry logic to gate pipeline"',
        description: 'Capture a work item',
        delay_before_ms: 1000,
        type_speed_ms: 25,
        wait_after_ms: 2500,
        output: [
          `${SAGE}✓${R} Idea captured:`,
          `  ${BONE}Add retry logic to gate pipeline${R}`,
          `  slug: ${ROSE}retry-gate-pipeline${R}`,
          `  path: ${DIM}.roko/prd/ideas/retry-gate-pipeline.md${R}`,
        ],
      },
      {
        terminal: 0,
        command: 'roko prd list',
        description: 'List all PRDs',
        delay_before_ms: 800,
        type_speed_ms: 30,
        wait_after_ms: 2000,
        output: [
          `${ROSE} PRD                            STATUS      TASKS${R}`,
          ` ${BONE}system-prompt-wiring${R}            ${SAGE}done${R}        8/8`,
          ` ${BONE}episode-logger${R}                  ${SAGE}done${R}        5/5`,
          ` ${BONE}process-supervisor${R}              ${SAGE}done${R}        6/6`,
          ` ${BONE}mcp-passthrough${R}                 ${SAGE}done${R}        4/4`,
          ` ${BONE}learning-feedback${R}               ${SAGE}done${R}       12/12`,
          ` ${BONE}interactive-tui${R}                 ${SAGE}done${R}       19/19`,
          ` ${BONE}http-control-plane${R}              ${SAGE}done${R}       14/14`,
          ` ${BONE}retry-gate-pipeline${R}             ${AMBER}idea${R}        ${DIM}—${R}`,
          '',
          ` ${DIM}8 PRDs · 7 done · 1 idea${R}`,
        ],
      },
      {
        terminal: 0,
        command: 'roko learn all',
        description: 'Inspect learning state',
        delay_before_ms: 800,
        type_speed_ms: 30,
        wait_after_ms: 3000,
        output: [
          `${ROSE}┌─ learning state ──────────────────────────────${R}`,
          `${ROSE}│${R} ${BOLD}Cascade Router${R}`,
          `${ROSE}│${R}   current model   ${BONE}claude-haiku-4-5${R}`,
          `${ROSE}│${R}   models tried     ${AMBER}4${R} (haiku: 62%, sonnet: 28%, opus: 8%, gemini: 2%)`,
          `${ROSE}│${R}   cost savings     ${SAGE}96.8%${R} vs naive`,
          `${ROSE}│${R}`,
          `${ROSE}│${R} ${BOLD}Gate Thresholds${R}  (adaptive EMA)`,
          `${ROSE}│${R}   compile          ${SAGE}0.98${R}`,
          `${ROSE}│${R}   test             ${SAGE}0.94${R}`,
          `${ROSE}│${R}   clippy           ${AMBER}0.91${R}`,
          `${ROSE}│${R}   diff             ${AMBER}0.87${R}`,
          `${ROSE}│${R}`,
          `${ROSE}│${R} ${BOLD}Efficiency${R}`,
          `${ROSE}│${R}   episodes         ${AMBER}847${R}`,
          `${ROSE}│${R}   avg cost/task    ${BONE}$0.017${R}`,
          `${ROSE}│${R}   c-factor         ${SAGE}0.847${R}`,
          `${ROSE}└──────────────────────────────────────────────${R}`,
        ],
      },
      {
        terminal: 0,
        command: 'roko doctor',
        description: 'Run diagnostics',
        delay_before_ms: 800,
        type_speed_ms: 30,
        wait_after_ms: 2500,
        output: [
          `${SAGE}✓${R} .roko/ directory`,
          `${SAGE}✓${R} roko.toml config`,
          `${SAGE}✓${R} signal log (1,247 entries)`,
          `${SAGE}✓${R} episode log (847 entries)`,
          `${SAGE}✓${R} executor state`,
          `${SAGE}✓${R} cascade router persisted`,
          `${SAGE}✓${R} gate thresholds persisted`,
          `${SAGE}✓${R} claude CLI available`,
          `${AMBER}!${R} ollama not running ${DIM}(optional)${R}`,
          '',
          `${SAGE}8/9 checks passed${R} — workspace healthy`,
        ],
      },
    ],
  },

  /* ═══════════════════════════════════════════════════════════
     3. BUILD — Prompt to validated code
     ═══════════════════════════════════════════════════════════ */
  {
    id: 'builder',
    title: 'Build',
    subtitle: 'Type a prompt. Roko builds it, validates with gates, shows cost.',
    panes: 1,
    labels: ['builder'],
    steps: [
      {
        terminal: 0,
        command: 'cd $(mktemp -d /tmp/roko-build-XXXX) && roko init -q',
        description: 'Create ephemeral workspace',
        delay_before_ms: 400,
        type_speed_ms: 22,
        wait_after_ms: 800,
        output: [
          `${SAGE}✓${R} Workspace ready at ${BONE}/tmp/roko-build-7b3a${R}`,
        ],
      },
      {
        terminal: 0,
        command: 'roko run "Build a CLI calculator in Rust"',
        description: 'Submit prompt to agent',
        delay_before_ms: 500,
        type_speed_ms: 22,
        wait_after_ms: 8000,
        output: [
          `${ROSE}▸ composing prompt${R} (9 layers)`,
          `  role: ${BONE}implementation${R}  model: ${BONE}claude-sonnet-4${R}`,
          '',
          `${ROSE}▸ dispatching agent${R}`,
          `  creating ${AMBER}src/main.rs${R}`,
          `  creating ${AMBER}Cargo.toml${R}`,
          `  creating ${AMBER}src/calculator.rs${R}`,
          `  ${DIM}342 tokens in · 1,204 tokens out${R}`,
          '',
          `${ROSE}▸ running gates${R}`,
          `  ${SAGE}✓${R} compile    ${DIM}1.2s${R}`,
          `  ${SAGE}✓${R} test       ${DIM}0.8s${R}`,
          `  ${SAGE}✓${R} clippy     ${DIM}0.6s${R}`,
          `  ${SAGE}✓${R} diff       ${DIM}0.1s${R}`,
          '',
          `${SAGE}✓ all gates passed${R}  cost: ${BONE}$0.024${R}  time: ${AMBER}4.1s${R}`,
        ],
      },
      {
        terminal: 0,
        command: 'hx src/main.rs',
        description: 'Inspect generated code',
        delay_before_ms: 1000,
        type_speed_ms: 28,
        wait_after_ms: 5000,
        output: helixChrome('src/main.rs', [
          `${LILAC}use${R} ${WHITE}std::io${R};`,
          `${LILAC}mod${R} ${WHITE}calculator${R};`,
          '',
          `${LILAC}fn${R} ${CYAN}main${R}() {`,
          `    ${WHITE}println!(${BONE}"roko-calc v0.1.0"${R}${WHITE});${R}`,
          `    ${LILAC}let${R} ${WHITE}mut input = String::new();${R}`,
          `    ${LILAC}loop${R} {`,
          `        ${WHITE}print!(${BONE}"> "${R}${WHITE});${R}`,
          `        ${WHITE}io::stdin().read_line(&mut input).unwrap();${R}`,
          `        ${LILAC}match${R} ${WHITE}calculator::eval(&input.trim()) {${R}`,
          `            ${SAGE}Ok${R}(v) => ${WHITE}println!(${BONE}"{v}"${R}${WHITE}),${R}`,
          `            ${ROSE}Err${R}(e) => ${WHITE}eprintln!(${BONE}"error: {e}"${R}${WHITE}),${R}`,
          `        }`,
          `        ${WHITE}input.clear();${R}`,
          `    }`,
          `}`,
        ], 'rust'),
      },
    ],
  },

  /* ═══════════════════════════════════════════════════════════
     4. COST RACE — Naive vs cascade routing
     ═══════════════════════════════════════════════════════════ */
  {
    id: 'race',
    title: 'Cost Race',
    subtitle: 'Same task, two approaches. Left: naive single-model. Right: cascade-routed.',
    panes: 2,
    labels: ['naive (single-model)', 'cascade (routed)'],
    steps: [
      {
        terminal: 0,
        command: 'cd $(mktemp -d /tmp/roko-naive-XXXX) && roko init -q',
        description: 'Create naive workspace',
        delay_before_ms: 400,
        type_speed_ms: 22,
        wait_after_ms: 600,
        output: [
          `${SAGE}✓${R} Workspace ready at ${BONE}/tmp/roko-naive-e2d8${R}`,
        ],
      },
      {
        terminal: 1,
        command: 'cd $(mktemp -d /tmp/roko-cascade-XXXX) && roko init -q',
        description: 'Create cascade workspace',
        delay_before_ms: 200,
        type_speed_ms: 22,
        wait_after_ms: 600,
        output: [
          `${SAGE}✓${R} Workspace ready at ${BONE}/tmp/roko-cascade-f1a9${R}`,
        ],
      },
      {
        terminal: 0,
        command: 'roko run "Implement a key-value store" --no-replan',
        description: 'Naive: single model, no replanning',
        delay_before_ms: 500,
        type_speed_ms: 22,
        wait_after_ms: 1000,
        output: [
          `${AMBER}▸ model:${R} claude-opus-4  ${DIM}(no routing)${R}`,
          '',
          `${AMBER}▸ dispatching${R}`,
          `  creating ${BONE}store.rs${R}  ${DIM}2,847 tokens${R}`,
          `  creating ${BONE}lib.rs${R}    ${DIM}1,203 tokens${R}`,
          `  creating ${BONE}main.rs${R}   ${DIM}891 tokens${R}`,
          '',
          `${AMBER}▸ gates${R}`,
          `  ${SAGE}✓${R} compile  ${SAGE}✓${R} test  ${SAGE}✓${R} clippy  ${SAGE}✓${R} diff`,
          '',
          `  cost: ${AMBER}$0.89${R}  tokens: ${AMBER}4,941${R}`,
        ],
      },
      {
        terminal: 1,
        command: 'roko run "Implement a key-value store"',
        description: 'Cascade: model routing + gate replan',
        delay_before_ms: 200,
        type_speed_ms: 22,
        wait_after_ms: 10000,
        output: [
          `${ROSE}▸ cascade routing${R}`,
          `  task type: ${BONE}implementation${R}`,
          `  selected:  ${SAGE}claude-haiku-4-5${R}  ${DIM}(96% cheaper)${R}`,
          '',
          `${ROSE}▸ dispatching${R}`,
          `  creating ${BONE}store.rs${R}  ${DIM}1,924 tokens${R}`,
          `  creating ${BONE}lib.rs${R}    ${DIM}847 tokens${R}`,
          `  creating ${BONE}main.rs${R}   ${DIM}612 tokens${R}`,
          '',
          `${ROSE}▸ gates${R}`,
          `  ${SAGE}✓${R} compile  ${SAGE}✓${R} test  ${SAGE}✓${R} clippy  ${SAGE}✓${R} diff`,
          '',
          `  cost: ${SAGE}$0.031${R}  tokens: ${SAGE}3,383${R}`,
          `  ${SAGE}saved 96.5% vs naive${R}`,
        ],
      },
    ],
  },

  /* ═══════════════════════════════════════════════════════════
     5. FLEET — Multi-agent coordination
     ═══════════════════════════════════════════════════════════ */
  {
    id: 'fleet',
    title: 'Fleet',
    subtitle: 'Multi-agent coordination. Inspect registered agents and workspace state.',
    panes: 2,
    labels: ['agents', 'status'],
    steps: [
      {
        terminal: 0,
        command: 'cd $(mktemp -d /tmp/roko-fleet-XXXX) && roko init -q',
        description: 'Create fleet workspace',
        delay_before_ms: 400,
        type_speed_ms: 22,
        wait_after_ms: 600,
        output: [
          `${SAGE}✓${R} Workspace ready at ${BONE}/tmp/roko-fleet-d3b7${R}`,
        ],
      },
      {
        terminal: 0,
        command: 'roko agent list',
        description: 'List registered agents',
        delay_before_ms: 500,
        type_speed_ms: 30,
        wait_after_ms: 2500,
        output: [
          `${ROSE} AGENT               DOMAIN         STATUS   MODEL${R}`,
          ` ${BONE}architect${R}            system-design  ${SAGE}active${R}   claude-sonnet-4`,
          ` ${BONE}implementer${R}          code           ${SAGE}active${R}   claude-haiku-4-5`,
          ` ${BONE}reviewer${R}             code-review    ${SAGE}active${R}   claude-haiku-4-5`,
          ` ${BONE}researcher${R}           research       ${DIM}idle${R}     perplexity`,
          ` ${BONE}tester${R}               testing        ${DIM}idle${R}     claude-haiku-4-5`,
          '',
          ` ${DIM}5 agents · 3 active · 2 idle${R}`,
        ],
      },
      {
        terminal: 1,
        command: 'roko status',
        description: 'Workspace overview',
        delay_before_ms: 300,
        type_speed_ms: 30,
        wait_after_ms: 2500,
        output: [
          `${ROSE}┌─ roko status ─────────────────────────────────${R}`,
          `${ROSE}│${R}  workspace    ${BONE}/tmp/roko-fleet-d3b7${R}`,
          `${ROSE}│${R}  version      ${BONE}0.9.2${R}`,
          `${ROSE}│${R}  signals      ${AMBER}1,247${R}`,
          `${ROSE}│${R}  episodes     ${AMBER}847${R}`,
          `${ROSE}│${R}  agents       ${SAGE}5 registered${R}  ${SAGE}3 active${R}`,
          `${ROSE}│${R}  gate pass    ${SAGE}93.4%${R}`,
          `${ROSE}│${R}  total cost   ${BONE}$1.42${R}`,
          `${ROSE}└──────────────────────────────────────────────${R}`,
        ],
      },
    ],
  },
];
