/* ================================================================
   WEBSOCKET — live stream connection + event handling
   ================================================================ */

import { state } from './state.js';
import { logReq, logAgent, toast } from './api.js';
import { depositPheromoneParticle } from './pheromones.js';
import { addInsightNode, graphNodes } from './graph.js';

export async function toggleWs() {
  if (state.ws && state.ws.readyState === WebSocket.OPEN) {
    state.ws.close(); state.ws = null; state.wsLive = false;
    document.getElementById('btn-ws').textContent = 'WS LIVE';
    toast('info', 'WS closed');
    return;
  }
  var wsBase = state.rpcUrl.replace(/^http:\/\//, 'ws://').replace(/^https:\/\//, 'wss://');
  // Strip trailing slash if present
  wsBase = wsBase.replace(/\/$/, '');
  var wsUrl = wsBase + '/api/ws?pheromones=true&insights=true';
  try {
    var ws = new WebSocket(wsUrl); state.ws = ws;
    ws.onopen = function() {
      state.wsLive = true;
      document.getElementById('btn-ws').textContent = 'WS OPEN';
      toast('ok', 'WS · connected to /api/ws');
      logAgent('act', 'ws', 'connected to /api/ws?pheromones=true&insights=true');
    };
    ws.onmessage = function(ev) {
      try {
        var msg = JSON.parse(ev.data);
        // Handle new WS protocol
        if (msg.type === 'connected') {
          logReq('sub', 'WS connected: ' + JSON.stringify(msg));
          return;
        }
        if (msg.type === 'lagged') {
          logReq('warn', 'WS lagged: channel=' + msg.channel + ' missed=' + msg.missed);
          toast('warn', 'WS lagged on ' + msg.channel + ': missed ' + msg.missed + ' events');
          return;
        }
        // Channel-based messages
        if (msg.channel === 'pheromone' && msg.data) {
          var p = msg.data;
          var kind = (p.kind || '').toLowerCase();
          var content = p.content || ('pheromone #' + (p.id || '?'));
          depositPheromoneParticle(kind, content, p.intensity || 0.7, p.id);
          if (p.decay_projection) {
            var newP = state.pheromones[state.pheromones.length - 1];
            if (newP) newP.decayProjection = p.decay_projection;
          }
          // WS pheromone events don't carry author — show kind + id + intensity
          logAgent(kind, 'pheromone', kind + ' #' + (p.id || '?') + ' deposited (i=' + (p.intensity||0).toFixed(2) + ')');
        } else if (msg.channel === 'insight' && msg.data) {
          var e = msg.data;
          // The WS insight event has "type" (posted/confirmed/challenged/stateTransition/decayed)
          // and "kind" (insight/heuristic/warning/etc for posted events)
          var eventType = (e.type || 'posted').toString().toLowerCase();
          if (eventType === 'posted') {
            var id = (e.id || '').replace(/^insight:/, '');
            if (id && !state.insights.has(id)) {
              var insKind = e.kind || 'insight';
              var content = e.content || '';
              var author = e.author || 'chain';
              state.insights.set(id, {id: id, kind: insKind, content: content, author: author, conf:0, chall:0, weight:1.0, createdAt:Date.now()});
              state.seenAuthors.add(author);
              addInsightNode(id, insKind, content);
              logAgent(insKind, author, content.slice(0, 90));
              state.postsLastMin.push(Date.now());
            }
          } else if (eventType === 'confirmed') {
            var id = (e.id || '').replace(/^insight:/, '');
            var by = e.by || 'chain';
            var n = graphNodes.find(function(x) { return x.id===id; }); if (n) { n.conf++; n.pulse=1; }
            state.confirmsCount++;
            state.seenAuthors.add(by);
            logAgent('confirm', by, 'confirmed insight ' + id.slice(0, 12) + '…');
          } else if (eventType === 'challenged') {
            var id = (e.id || '').replace(/^insight:/, '');
            var by = e.by || 'chain';
            var n = graphNodes.find(function(x) { return x.id===id; }); if (n) { n.chall++; n.pulse=1; }
            state.challengesCount++;
            state.seenAuthors.add(by);
            logAgent('challenge', by, 'challenged insight ' + id.slice(0, 12) + '…');
          } else if (eventType === 'statetransition') {
            logAgent('observe', 'chain', 'insight ' + (e.id||'').slice(0,12) + '… state: ' + (e.from||'?') + ' → ' + (e.to||'?'));
          }
        }
        // Also handle legacy JSON-RPC format for backwards compatibility
        if (msg.method === 'chain_pheromoneEvent' && msg.params) {
          var lp = msg.params.result || msg.params;
          var lkind = (lp.kind || '').toLowerCase();
          var lcontent = lp.content || ('pheromone #' + (lp.id || '?'));
          depositPheromoneParticle(lkind, lcontent, lp.intensity || 0.7, lp.id);
          logAgent(lkind, 'ws', lkind + ' deposited (i=' + (lp.intensity||0).toFixed(2) + ')');
        } else if (msg.method === 'chain_insightEvent' && msg.params) {
          var le = msg.params.result || msg.params;
          var lvariant = (le.kind || le.event || 'posted').toString().toLowerCase();
          if (lvariant === 'posted') {
            var lid = (le.id || '').replace(/^insight:/, '');
            if (lid && !state.insights.has(lid)) {
              var linsKind = le.insightKind || le.kind || 'insight';
              var lcontent2 = le.content || '';
              var lauthor = le.author || 'chain';
              state.insights.set(lid, {id: lid, kind: linsKind, content: lcontent2, author: lauthor, conf:0, chall:0, weight:1.0, createdAt:Date.now()});
              state.seenAuthors.add(lauthor);
              addInsightNode(lid, linsKind, lcontent2);
              logAgent('observe', lauthor, lcontent2.slice(0, 80));
              state.postsLastMin.push(Date.now());
            }
          } else if (lvariant === 'confirmed') {
            var lid2 = (le.id || '').replace(/^insight:/, '');
            var ln = graphNodes.find(function(x) { return x.id===lid2; }); if (ln) { ln.conf++; ln.pulse=1; }
            state.confirmsCount++;
          } else if (lvariant === 'challenged') {
            var lid3 = (le.id || '').replace(/^insight:/, '');
            var ln2 = graphNodes.find(function(x) { return x.id===lid3; }); if (ln2) { ln2.chall++; ln2.pulse=1; }
            state.challengesCount++;
          }
        } else if (msg.result && msg.result.subscriptionId) {
          logReq('sub', 'subscribed: ' + msg.result.subscriptionId);
        }
      } catch(parseErr) {}
    };
    ws.onerror = function() { logReq('err', 'WS error'); };
    ws.onclose = function() {
      state.wsLive = false; state.ws = null;
      document.getElementById('btn-ws').textContent = 'WS LIVE';
    };
  } catch (e) { toast('err', 'WS: ' + e.message); }
}
