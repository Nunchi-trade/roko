/* ================================================================
   API — rpc(), api(), apiPost(), logging, toast, formatters
   ================================================================ */

import { state } from './state.js';

var rpcId = 1;

/* ---------- JSON-RPC call ---------- */
export async function rpc(method, params) {
  state.rpc.total++;
  var id = rpcId++;
  logReq('rpc', method + ' #' + id);
  var t0 = performance.now();
  try {
    var resp = await fetch(state.rpcUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', method: method, params: params || [], id: id }),
    });
    var json = await resp.json();
    var ms = performance.now() - t0;
    logReq('ok', method + ' ' + Math.round(ms) + 'ms');
    return { result: json.result, error: json.error, ms: ms };
  } catch (e) {
    state.rpc.errors++;
    logReq('err', method + ' FAILED: ' + e.message);
    throw e;
  }
}

/* ---------- REST API GET ---------- */
export async function api(path) {
  state.rpc.total++;
  logReq('api', 'GET /api' + path);
  var t0 = performance.now();
  try {
    var resp = await fetch(state.rpcUrl.replace(/\/$/, '') + '/api' + path);
    var json = await resp.json();
    var ms = performance.now() - t0;
    logReq('ok', 'GET /api' + path + ' ' + Math.round(ms) + 'ms');
    return { data: json, ms: ms };
  } catch (e) {
    state.rpc.errors++;
    logReq('err', 'GET /api' + path + ' FAILED: ' + e.message);
    throw e;
  }
}

/* ---------- REST API POST ---------- */
export async function apiPost(path, body) {
  state.rpc.total++;
  logReq('api', 'POST /api' + path);
  var t0 = performance.now();
  try {
    var resp = await fetch(state.rpcUrl.replace(/\/$/, '') + '/api' + path, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    var json = await resp.json();
    var ms = performance.now() - t0;
    logReq('ok', 'POST /api' + path + ' ' + Math.round(ms) + 'ms');
    return { data: json, ms: ms };
  } catch (e) {
    state.rpc.errors++;
    logReq('err', 'POST /api' + path + ' FAILED: ' + e.message);
    throw e;
  }
}

/* ---------- Render callbacks (wired by main.js to avoid circular deps) ---------- */
var _renderLog = null;
var _renderAgent = null;
export function onRenderLog(fn) { _renderLog = fn; }
export function onRenderAgent(fn) { _renderAgent = fn; }

/* ---------- Request log ---------- */
export function logReq(lv, msg) {
  state.requestLog.push({ ts: Date.now(), lv: lv, msg: msg });
  if (state.requestLog.length > 200) state.requestLog.shift();
  if (_renderLog) _renderLog();
}

/* ---------- Agent log ---------- */
export function logAgent(type, author, msg) {
  state.agentLog.push({ ts: Date.now(), type: type, author: author, msg: msg });
  if (state.agentLog.length > 120) state.agentLog.shift();
  state.seenAuthors.add(author);
  if (_renderAgent) _renderAgent();
}

/* ---------- Toasts ---------- */
export function toast(kind, msg) {
  var el = document.createElement('div');
  el.className = 'toast ' + kind;
  el.textContent = msg;
  document.getElementById('toasts').appendChild(el);
  setTimeout(function() { el.remove(); }, 3400);
}

/* ---------- Format helpers ---------- */
export var fmtTs = function(ms) { return new Date(ms).toLocaleTimeString('en-US', {hour12:false}); };
export var shortHash = function(h) {
  if (!h) return '';
  var s = h.startsWith('insight:') ? h.slice(8) : h;
  return s.length > 14 ? s.slice(0,8) + '…' + s.slice(-4) : s;
};

export var parseHexU64 = function(s) { if (!s) return 0; var h = s.startsWith('0x') ? s.slice(2) : s; return parseInt(h, 16); };
export var parseHexBig = function(s) { if (!s) return 0n; var h = s.startsWith('0x') ? s.slice(2) : s; return BigInt('0x'+h); };
export var weiToGwei = function(wei) { return Number(wei) / 1e9; };
