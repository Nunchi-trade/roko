import { useEffect, useRef, useState, useCallback } from 'react';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface JobEvent {
  type: 'posted' | 'assigned' | 'submitted' | 'vote' | 'resolved';
  id: number;
  specHash?: string;
  bounty?: string;
  poster?: string;
  worker?: string;
  resultHash?: string;
  voter?: string;
  approve?: boolean;
  voteCount?: number;
  accepted?: boolean;
  payout?: string;
}

export interface AgentChainEvent {
  type: 'registered' | 'reputation' | 'heartbeat';
  address: string;
  capabilities?: string;
  old?: number;
  new?: number;
  tier?: string;
  block?: number;
}

export interface JobState {
  id: number;
  specHash: string;
  bounty: string;
  poster: string;
  worker?: string;
  resultHash?: string;
  state: 'funded' | 'assigned' | 'submitted' | 'resolved';
  accepted?: boolean;
}

export interface AgentInfo {
  address: string;
  name: string;
  role: 'poster' | 'worker';
  reputation: number;
  tier: string;
  lastHeartbeat: number;
}

export interface VoteState {
  voters: { address: string; approve: boolean | null }[];
  verdict: 'pending' | 'approved' | 'rejected';
}

export interface ChainTx {
  block: number;
  fn: string;
  amount?: string;
  type: 'fund' | 'advance' | 'vote';
}

export interface UseJobEventsReturn {
  connected: boolean;
  jobs: JobState[];
  agents: AgentInfo[];
  votes: VoteState;
  events: (JobEvent | AgentChainEvent)[];
  chainTxs: ChainTx[];
  /** Drive the demo lifecycle: register agents → post job → assign → start → complete. */
  runDemo: () => Promise<void>;
  /** Whether the demo is currently running. */
  demoRunning: boolean;
}

// ---------------------------------------------------------------------------
// WebSocket message types from mirage-rs
// ---------------------------------------------------------------------------

interface WsConnectedMsg {
  type: 'connected';
  jobs: boolean;
  agents: boolean;
}

interface WsChannelMsg {
  channel: 'job' | 'agent';
  data: JobEvent | AgentChainEvent;
  type?: 'lagged';
  missed?: number;
}

type WsMessage = WsConnectedMsg | WsChannelMsg;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

const MIRAGE_HOST = 'localhost:8545';
const WS_URL = `ws://${MIRAGE_HOST}/api/ws/jobs?jobs=true&agents=true`;
const MAX_EVENTS = 200;

/** Exponential backoff: 1s, 2s, 4s, 8s, … capped at 30s */
const INITIAL_BACKOFF_MS = 1_000;
const MAX_BACKOFF_MS = 30_000;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Derive the next job state from the previous state + incoming event */
function applyJobEvent(jobs: JobState[], ev: JobEvent): JobState[] {
  const existing = jobs.find(j => j.id === ev.id);

  if (ev.type === 'posted') {
    if (existing) return jobs;
    const next: JobState = {
      id: ev.id,
      specHash: ev.specHash ?? '',
      bounty: ev.bounty ?? '0',
      poster: ev.poster ?? '',
      state: 'funded',
    };
    return [...jobs, next];
  }

  if (!existing) return jobs;

  return jobs.map(j => {
    if (j.id !== ev.id) return j;
    switch (ev.type) {
      case 'assigned':
        return { ...j, worker: ev.worker, state: 'assigned' as const };
      case 'submitted':
        return { ...j, resultHash: ev.resultHash, state: 'submitted' as const };
      case 'resolved':
        return { ...j, accepted: ev.accepted, state: 'resolved' as const };
      default:
        return j;
    }
  });
}

/** Derive the next agent list from the previous list + incoming event */
function applyAgentEvent(agents: AgentInfo[], ev: AgentChainEvent): AgentInfo[] {
  const existing = agents.find(a => a.address === ev.address);

  if (ev.type === 'registered') {
    if (existing) return agents;
    // Derive role from capabilities field (mirage sends role there)
    const role: 'poster' | 'worker' = ev.capabilities === 'poster' ? 'poster' : 'worker';
    // Derive name from address: "agent-alpha" → "ALPHA", fallback to truncated id
    const rawId = ev.address;
    const name = rawId.startsWith('agent-')
      ? rawId.slice(6).toUpperCase()
      : rawId.slice(0, 8).toUpperCase();
    const next: AgentInfo = {
      address: ev.address,
      name,
      role,
      reputation: 500_000,
      tier: ev.tier ?? 'Standard',
      lastHeartbeat: Date.now(),
    };
    return [...agents, next];
  }

  if (!existing) return agents;

  return agents.map(a => {
    if (a.address !== ev.address) return a;
    switch (ev.type) {
      case 'reputation':
        return {
          ...a,
          reputation: ev.new ?? a.reputation,
          tier: ev.tier ?? a.tier,
        };
      case 'heartbeat':
        return { ...a, lastHeartbeat: Date.now() };
      default:
        return a;
    }
  });
}

/** Map a job event type to a ChainTx */
function jobEventToChainTx(ev: JobEvent, block: number): ChainTx | null {
  switch (ev.type) {
    case 'posted':
      return { block, fn: 'postJob', amount: ev.bounty, type: 'fund' };
    case 'assigned':
      return { block, fn: 'assignJob', type: 'advance' };
    case 'submitted':
      return { block, fn: 'submitResult', type: 'advance' };
    case 'vote':
      return { block, fn: 'castVote', type: 'vote' };
    case 'resolved':
      return { block, fn: 'resolveJob', amount: ev.payout, type: 'advance' };
    default:
      return null;
  }
}

// ---------------------------------------------------------------------------
// useJobEvents — real WebSocket hook
// ---------------------------------------------------------------------------

export function useJobEvents(): UseJobEventsReturn {
  const [connected, setConnected] = useState(false);
  const [jobs, setJobs] = useState<JobState[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [votes, setVotes] = useState<VoteState>({ voters: [], verdict: 'pending' });
  const [events, setEvents] = useState<(JobEvent | AgentChainEvent)[]>([]);
  const [chainTxs, setChainTxs] = useState<ChainTx[]>([]);

  const wsRef = useRef<WebSocket | null>(null);
  const backoffRef = useRef(INITIAL_BACKOFF_MS);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mountedRef = useRef(true);
  // Incrementing synthetic block counter for chainTxs
  const blockRef = useRef(1);

  const connect = useCallback(() => {
    // Clean up any existing connection
    if (wsRef.current) {
      wsRef.current.close();
      wsRef.current = null;
    }

    const ws = new WebSocket(WS_URL);
    wsRef.current = ws;

    ws.onopen = () => {
      if (!mountedRef.current) return;
      backoffRef.current = INITIAL_BACKOFF_MS;
      // connected state is set when we receive the server confirmation message
    };

    ws.onmessage = (e: MessageEvent) => {
      if (!mountedRef.current) return;
      let msg: WsMessage;
      try {
        msg = JSON.parse(e.data as string);
      } catch {
        return;
      }

      // Connection confirmation from mirage
      if ('type' in msg && msg.type === 'connected') {
        setConnected(true);
        return;
      }

      const channelMsg = msg as WsChannelMsg;

      // Backpressure warning
      if (channelMsg.type === 'lagged') {
        console.warn(`[useJobEvents] backpressure: missed ${channelMsg.missed} events on ${channelMsg.channel}`);
        return;
      }

      if (channelMsg.channel === 'job') {
        const data = channelMsg.data as JobEvent;

        setJobs(prev => applyJobEvent(prev, data));

        // Update vote state when a vote event arrives
        if (data.type === 'vote' && data.voter !== undefined) {
          setVotes(prev => {
            const existing = prev.voters.find(v => v.address === data.voter);
            const voters = existing
              ? prev.voters.map(v =>
                  v.address === data.voter ? { ...v, approve: data.approve ?? null } : v,
                )
              : [...prev.voters, { address: data.voter!, approve: data.approve ?? null }];

            const approvals = voters.filter(v => v.approve === true).length;
            const rejections = voters.filter(v => v.approve === false).length;
            const verdict: VoteState['verdict'] =
              approvals > voters.length / 2
                ? 'approved'
                : rejections > voters.length / 2
                  ? 'rejected'
                  : 'pending';

            return { voters, verdict };
          });
        }

        // Append to events (capped at MAX_EVENTS)
        setEvents(prev => {
          const next = [...prev, data];
          return next.length > MAX_EVENTS ? next.slice(-MAX_EVENTS) : next;
        });

        // Build a ChainTx for this event
        const tx = jobEventToChainTx(data, blockRef.current++);
        if (tx) {
          setChainTxs(prev => [...prev, tx]);
        }
      } else if (channelMsg.channel === 'agent') {
        const data = channelMsg.data as AgentChainEvent;

        setAgents(prev => applyAgentEvent(prev, data));

        // Append to events (capped at MAX_EVENTS)
        setEvents(prev => {
          const next = [...prev, data];
          return next.length > MAX_EVENTS ? next.slice(-MAX_EVENTS) : next;
        });
      }
    };

    ws.onclose = () => {
      if (!mountedRef.current) return;
      wsRef.current = null;
      setConnected(false);

      // Schedule reconnect with exponential backoff
      const delay = backoffRef.current;
      backoffRef.current = Math.min(delay * 2, MAX_BACKOFF_MS);
      reconnectTimer.current = setTimeout(() => {
        if (mountedRef.current) connect();
      }, delay);
    };

    ws.onerror = () => {
      if (!mountedRef.current) return;
      // onclose fires after onerror, so reconnect is handled there
    };
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    connect();

    return () => {
      mountedRef.current = false;
      if (reconnectTimer.current) clearTimeout(reconnectTimer.current);
      wsRef.current?.close();
      wsRef.current = null;
    };
  }, [connect]);

  // ---------------------------------------------------------------------------
  // runDemo — drive lifecycle via mirage HTTP API
  // ---------------------------------------------------------------------------

  const [demoRunning, setDemoRunning] = useState(false);

  const runDemo = useCallback(async () => {
    if (demoRunning) return;
    setDemoRunning(true);

    try {
      const base = `http://${MIRAGE_HOST}`;
      const post = async (path: string, body: Record<string, unknown>) => {
        const res = await fetch(`${base}/api${path}`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
        });
        return res.json();
      };

      // Phase 1: Register agents
      await post('/agents', { id: 'agent-alpha', role: 'poster' });
      await new Promise(r => setTimeout(r, 400));

      await post('/agents', { id: 'agent-beta', role: 'worker' });
      await new Promise(r => setTimeout(r, 600));

      // Phase 2: Post job (creates a task → fires "posted" event)
      const taskRes = await post('/tasks', {
        title: 'Research Uniswap V4 Gas Optimization',
        description: 'Analyze hook gas costs, compare V3 vs V4 routing, produce optimization report',
        kind: 'research',
        creator: 'agent-alpha',
        tags: ['defi', 'uniswap', 'gas'],
        stake_wei: 50,
      });
      const taskId = taskRes?.id;
      if (!taskId) {
        console.warn('[runDemo] failed to create task:', taskRes);
        return;
      }
      await new Promise(r => setTimeout(r, 1200));

      // Phase 3: Assign to Beta (fires "assigned")
      await post(`/tasks/${taskId}/assign`, { assignee: 'agent-beta' });
      await new Promise(r => setTimeout(r, 1500));

      // Phase 4: Start work (fires "submitted" — mapped from Started)
      await post(`/tasks/${taskId}/start`, {});
      await new Promise(r => setTimeout(r, 2000));

      // Phase 5: Complete with result (fires "resolved")
      await post(`/tasks/${taskId}/complete`, {
        summary: 'V4 hooks reduce gas by 23% on average vs V3 multi-hop routing',
        artifacts: [
          { name: 'gas-report.md', kind: 'report', url: 'ipfs://QmGasReport...' },
          { name: 'benchmark-data.json', kind: 'data', url: 'ipfs://QmBenchData...' },
        ],
      });
    } catch (err) {
      console.error('[runDemo] error:', err);
    } finally {
      setDemoRunning(false);
    }
  }, [demoRunning]);

  return { connected, jobs, agents, votes, events, chainTxs, runDemo, demoRunning };
}
