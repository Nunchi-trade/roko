import assert from 'node:assert/strict';

import {
  DEFAULT_REMOTE_BASE,
  describeAgentSources,
  deriveRemoteBases,
  mergeAgentSources,
  normalizeRemoteBase,
  selectTransportPath,
} from './agent_registry.js';

function parseArgs(argv) {
  var args = { check: false, base: '' };
  for (var i = 0; i < argv.length; i++) {
    if (argv[i] === '--check') args.check = true;
    if (argv[i] === '--base' && argv[i + 1]) args.base = argv[i + 1];
  }
  return args;
}

function runStaticChecks() {
  assert.equal(
    normalizeRemoteBase('http://127.0.0.1:8545/dashboard/'),
    DEFAULT_REMOTE_BASE
  );
  assert.equal(
    normalizeRemoteBase('https://demo.example.test/relay'),
    'https://demo.example.test'
  );

  var bases = deriveRemoteBases('https://demo.example.test/dashboard/');
  assert.equal(bases.apiBase, 'https://demo.example.test/api');
  assert.equal(bases.relayBase, 'https://demo.example.test/relay');
  assert.equal(bases.wsUrl, 'wss://demo.example.test/api/ws');

  var merged = mergeAgentSources(
    [{
      key: 'passport:7',
      agentId: 'wallet-agent',
      displayName: 'wallet-agent',
      passportId: 7,
      owner: '0xabc',
      cardUri: 'https://demo.example.test/relay/cards/wallet-agent',
      directEndpoint: 'http://127.0.0.1:8081',
      sources: ['identity'],
    }],
    [{
      key: 'relay:wallet-agent',
      agentId: 'wallet-agent',
      displayName: 'wallet-agent',
      relayAgentId: 'wallet-agent',
      cardUri: 'https://demo.example.test/relay/cards/wallet-agent',
      relayAvailable: true,
      relayConnected: true,
      relayBacked: true,
      sources: ['relay'],
    }, {
      key: 'relay:laptop-agent',
      agentId: 'laptop-agent',
      displayName: 'laptop-agent',
      relayAgentId: 'laptop-agent',
      relayAvailable: true,
      relayConnected: true,
      relayBacked: true,
      directEndpoint: 'http://127.0.0.1:8099',
      sources: ['relay'],
    }]
  );

  assert.equal(merged.length, 2);
  var walletAgent = merged.find(function(agent) { return agent.agentId === 'wallet-agent'; });
  var laptopAgent = merged.find(function(agent) { return agent.agentId === 'laptop-agent'; });
  assert.ok(walletAgent, 'wallet-agent should be present');
  assert.ok(laptopAgent, 'laptop-agent should be present');
  assert.equal(describeAgentSources(walletAgent), 'identity + relay');
  assert.equal(selectTransportPath(walletAgent, 'auto', 'https://demo.example.test').mode, 'relay');
  assert.equal(selectTransportPath(laptopAgent, 'relay', 'https://demo.example.test').mode, 'relay');
  assert.equal(selectTransportPath(laptopAgent, 'direct', 'http://127.0.0.1:8545').mode, 'direct');
}

async function runLiveChecks(base) {
  var normalized = normalizeRemoteBase(base);
  var bases = deriveRemoteBases(normalized);

  var rpc = await fetch(bases.rpcUrl, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_blockNumber', params: [] }),
  });
  assert.equal(rpc.ok, true, 'eth_blockNumber failed');
  var rpcJson = await rpc.json();
  assert.equal(typeof rpcJson.result, 'string', 'eth_blockNumber result missing');

  var relay = await fetch(bases.relayBase + '/health');
  assert.equal(relay.ok, true, 'relay health failed');

  return {
    remoteBase: normalized,
    blockNumber: rpcJson.result,
    relayHealth: await relay.text(),
  };
}

async function main() {
  var args = parseArgs(process.argv.slice(2));
  if (args.check) {
    runStaticChecks();
    console.log(JSON.stringify({ ok: true, mode: 'static-check' }));
    return;
  }

  if (args.base) {
    runStaticChecks();
    var live = await runLiveChecks(args.base);
    console.log(JSON.stringify({ ok: true, mode: 'live-check', live: live }, null, 2));
    return;
  }

  runStaticChecks();
  console.log(JSON.stringify({ ok: true, mode: 'static-check' }));
}

main().catch(function(error) {
  console.error(error && error.stack ? error.stack : String(error));
  process.exitCode = 1;
});
