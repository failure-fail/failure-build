#!/usr/bin/env node
'use strict';

const http = require('http');
const crypto = require('crypto');
const { spawn, execFile } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');
const readline = require('readline');

const PROTOCOL_VERSION = '2025-03-26';
const HOST = process.env.FAILURE_MCP_BIND || '127.0.0.1';
const PORT = Number(process.env.FAILURE_MCP_PORT || 2420);
const ENABLE_TUNNEL = process.env.FAILURE_MCP_TUNNEL !== '0';
const TOKEN = process.env.FAILURE_MCP_TOKEN || crypto.randomBytes(24).toString('base64url');
const MAX_BODY = 8 * 1024 * 1024;

function argValue(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

const failureBin = argValue('--failure-bin') || process.env.FAILURE_BIN || 'failure';

function log(message) {
  process.stderr.write(`[failure-mcp] ${message}\n`);
}

function pretty(value) {
  return JSON.stringify(value, null, 2);
}

function result(value, isError = false) {
  return {
    content: [{ type: 'text', text: typeof value === 'string' ? value : pretty(value) }],
    isError,
  };
}

function rpcError(id, code, message, data) {
  const error = { code, message };
  if (data !== undefined) error.data = data;
  return { jsonrpc: '2.0', id: id ?? null, error };
}

class AcpBridge {
  constructor(binary) {
    this.binary = binary;
    this.child = null;
    this.nextId = 1;
    this.pending = new Map();
    this.notifications = [];
    this.initialized = false;
  }

  async start() {
    if (this.child && !this.child.killed) return;
    await new Promise((resolve, reject) => {
      const child = spawn(this.binary, ['agent', '--always-approve', 'stdio'], {
        stdio: ['pipe', 'pipe', 'pipe'],
        env: { ...process.env, FAILURE_MCP_CHILD: '1', FAILURE_AGENT_DASHBOARD: '0' },
      });
      this.child = child;
      readline.createInterface({ input: child.stdout }).on('line', (line) => this.onLine(line));
      child.stderr.on('data', (chunk) => process.stderr.write(chunk));
      child.once('spawn', resolve);
      child.once('error', reject);
      child.once('exit', (code, signal) => {
        const error = new Error(`Failure ACP process exited (code=${code}, signal=${signal})`);
        for (const pending of this.pending.values()) pending.reject(error);
        this.pending.clear();
        this.child = null;
        this.initialized = false;
      });
    });
  }

  onLine(line) {
    let message;
    try { message = JSON.parse(line); } catch { return; }
    const key = message.id === undefined ? null : String(message.id);
    if (key && this.pending.has(key)) {
      const pending = this.pending.get(key);
      this.pending.delete(key);
      clearTimeout(pending.timer);
      pending.resolve(message);
      return;
    }
    this.notifications.push(message);
    if (this.notifications.length > 1000) this.notifications.shift();
  }

  async request(method, params = {}, timeoutMs = 30 * 60 * 1000) {
    await this.start();
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(String(id));
        reject(new Error(`ACP request timed out: ${method}`));
      }, timeoutMs);
      this.pending.set(String(id), { resolve, reject, timer });
      this.child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
    });
  }

  async initialize() {
    if (this.initialized) return;
    const response = await this.request('initialize', {
      protocolVersion: 1,
      clientCapabilities: {
        fs: { readTextFile: true, writeTextFile: true },
        terminal: true,
      },
      clientInfo: { name: 'failure-remote-mcp', version: '0.1.0' },
      _meta: { clientIdentifier: 'failure-remote-mcp', clientType: 'generic' },
    }, 60_000);
    if (response.error) throw new Error(`Failure initialize failed: ${pretty(response.error)}`);
    this.initialized = true;
  }

  drainNotifications() {
    return this.notifications.splice(0, this.notifications.length);
  }

  stop() {
    try { this.child?.kill(); } catch {}
  }
}

const acp = new AcpBridge(failureBin);

const tools = [
  {
    name: 'failure_rpc',
    description: 'Call any Failure ACP JSON-RPC method. Use this for complete access to the built-in Failure agent API.',
    inputSchema: {
      type: 'object',
      required: ['method'],
      properties: {
        method: { type: 'string' },
        params: { type: 'object', additionalProperties: true },
        timeout_ms: { type: 'integer', minimum: 1, maximum: 3600000 },
        initialize_first: { type: 'boolean', default: true },
      },
      additionalProperties: false,
    },
  },
  {
    name: 'failure_new_chat',
    description: 'Create a new persistent Failure chat in a workspace.',
    inputSchema: {
      type: 'object',
      properties: {
        cwd: { type: 'string' },
        meta: { type: 'object', additionalProperties: true },
      },
      additionalProperties: false,
    },
  },
  {
    name: 'failure_continue_chat',
    description: 'Load an existing Failure chat by session ID.',
    inputSchema: {
      type: 'object',
      required: ['session_id'],
      properties: {
        session_id: { type: 'string' },
        cwd: { type: 'string' },
        meta: { type: 'object', additionalProperties: true },
      },
      additionalProperties: false,
    },
  },
  {
    name: 'failure_send_message',
    description: 'Send a message to a Failure chat. The agent can use its normal file, terminal, search, and coding tools.',
    inputSchema: {
      type: 'object',
      required: ['session_id', 'message'],
      properties: {
        session_id: { type: 'string' },
        message: { type: 'string' },
        meta: { type: 'object', additionalProperties: true },
        timeout_ms: { type: 'integer', minimum: 1, maximum: 3600000 },
      },
      additionalProperties: false,
    },
  },
  {
    name: 'failure_list_sessions',
    description: 'List saved Failure chats and sessions.',
    inputSchema: { type: 'object', properties: {}, additionalProperties: false },
  },
  {
    name: 'failure_status',
    description: 'Show the remote bridge status and optionally return buffered agent notifications.',
    inputSchema: {
      type: 'object',
      properties: { include_notifications: { type: 'boolean', default: false } },
      additionalProperties: false,
    },
  },
];

function runFailure(args, timeoutMs = 60_000) {
  return new Promise((resolve, reject) => {
    execFile(failureBin, args, {
      timeout: timeoutMs,
      maxBuffer: 32 * 1024 * 1024,
      env: { ...process.env, FAILURE_MCP_CHILD: '1' },
    }, (error, stdout, stderr) => {
      if (error?.killed) return reject(new Error(`Failure command timed out: ${args.join(' ')}`));
      resolve({ exitCode: typeof error?.code === 'number' ? error.code : 0, stdout, stderr });
    });
  });
}

async function callTool(name, args) {
  switch (name) {
    case 'failure_rpc': {
      if (args.initialize_first !== false && args.method !== 'initialize') await acp.initialize();
      const response = await acp.request(args.method, args.params || {}, args.timeout_ms || 1800000);
      return result({ response, notifications: acp.drainNotifications() }, Boolean(response.error));
    }
    case 'failure_new_chat': {
      await acp.initialize();
      const response = await acp.request('session/new', {
        cwd: path.resolve(args.cwd || process.cwd()),
        mcpServers: [],
        _meta: args.meta || {},
      });
      return result({ response, notifications: acp.drainNotifications() }, Boolean(response.error));
    }
    case 'failure_continue_chat': {
      await acp.initialize();
      const response = await acp.request('session/load', {
        sessionId: args.session_id,
        cwd: path.resolve(args.cwd || process.cwd()),
        mcpServers: [],
        _meta: args.meta || {},
      });
      return result({ response, notifications: acp.drainNotifications() }, Boolean(response.error));
    }
    case 'failure_send_message': {
      await acp.initialize();
      const response = await acp.request('session/prompt', {
        sessionId: args.session_id,
        prompt: [{ type: 'text', text: args.message }],
        _meta: args.meta || {},
      }, args.timeout_ms || 1800000);
      return result({ response, notifications: acp.drainNotifications() }, Boolean(response.error));
    }
    case 'failure_list_sessions':
      return result(await runFailure(['sessions', '--json']));
    case 'failure_status':
      return result({
        ok: true,
        pid: process.pid,
        localUrl: `http://127.0.0.1:${PORT}/mcp`,
        tunnelEnabled: ENABLE_TUNNEL,
        acpRunning: Boolean(acp.child && !acp.child.killed),
        hostname: os.hostname(),
        cwd: process.cwd(),
        notifications: args.include_notifications ? acp.drainNotifications() : undefined,
      });
    default:
      return result(`Unknown tool: ${name}`, true);
  }
}

async function handleRpc(message) {
  const id = message?.id;
  const method = message?.method;
  const params = message?.params || {};
  if (!method) return rpcError(id, -32600, 'Invalid Request');

  try {
    if (method === 'initialize') {
      return {
        jsonrpc: '2.0',
        id,
        result: {
          protocolVersion: params.protocolVersion || PROTOCOL_VERSION,
          capabilities: { tools: { listChanged: false } },
          serverInfo: { name: 'failure-build-remote', version: '0.1.0' },
          instructions: 'Use the chat tools for normal operation or failure_rpc for raw ACP access.',
        },
      };
    }
    if (method === 'ping') return { jsonrpc: '2.0', id, result: {} };
    if (method === 'tools/list') return { jsonrpc: '2.0', id, result: { tools } };
    if (method === 'tools/call') {
      return { jsonrpc: '2.0', id, result: await callTool(params.name, params.arguments || {}) };
    }
    if (method.startsWith('notifications/')) return null;
    return rpcError(id, -32601, `Method not found: ${method}`);
  } catch (error) {
    return rpcError(id, -32000, error.message);
  }
}

function authorized(request) {
  const auth = request.headers.authorization;
  if (auth === `Bearer ${TOKEN}`) return true;
  try {
    return new URL(request.url, 'http://localhost').searchParams.get('token') === TOKEN;
  } catch {
    return false;
  }
}

function cors(response) {
  response.setHeader('Access-Control-Allow-Origin', '*');
  response.setHeader('Access-Control-Allow-Headers', 'authorization, content-type, mcp-session-id');
  response.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
}

const server = http.createServer((request, response) => {
  cors(response);
  if (request.method === 'OPTIONS') {
    response.writeHead(204);
    response.end();
    return;
  }
  if (request.url === '/health') {
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ ok: true }));
    return;
  }
  if (!authorized(request)) {
    response.writeHead(401, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ error: 'Missing or invalid Failure MCP token' }));
    return;
  }
  if (request.method === 'GET') {
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ name: 'Failure Build Remote MCP', endpoint: '/mcp', transport: 'streamable-http' }));
    return;
  }
  if (request.method !== 'POST') {
    response.writeHead(405);
    response.end();
    return;
  }

  let size = 0;
  const chunks = [];
  request.on('data', (chunk) => {
    size += chunk.length;
    if (size > MAX_BODY) request.destroy(new Error('Request too large'));
    else chunks.push(chunk);
  });
  request.on('end', async () => {
    let payload;
    try { payload = JSON.parse(Buffer.concat(chunks).toString('utf8')); }
    catch { payload = null; }
    if (!payload) {
      response.writeHead(400, { 'content-type': 'application/json' });
      response.end(JSON.stringify(rpcError(null, -32700, 'Parse error')));
      return;
    }
    const batch = Array.isArray(payload);
    const messages = batch ? payload : [payload];
    const replies = (await Promise.all(messages.map(handleRpc))).filter(Boolean);
    if (!replies.length) {
      response.writeHead(202);
      response.end();
      return;
    }
    response.writeHead(200, {
      'content-type': 'application/json',
      'mcp-session-id': request.headers['mcp-session-id'] || `failure-${process.pid}`,
    });
    response.end(JSON.stringify(batch ? replies : replies[0]));
  });
});

let tunnel = null;

function saveState(publicUrl) {
  const dir = path.join(os.homedir(), '.failure');
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, 'mcp.json'), JSON.stringify({
    pid: process.pid,
    token: TOKEN,
    localUrl: `http://127.0.0.1:${PORT}/mcp?token=${encodeURIComponent(TOKEN)}`,
    publicUrl: publicUrl || null,
    startedAt: new Date().toISOString(),
  }, null, 2), { mode: 0o600 });
}

function startTunnel() {
  if (!ENABLE_TUNNEL) return;
  tunnel = spawn(process.env.CLOUDFLARED_BIN || 'cloudflared', [
    'tunnel', '--no-autoupdate', '--url', `http://127.0.0.1:${PORT}`,
  ], { stdio: ['ignore', 'pipe', 'pipe'] });
  let announced = false;
  const inspect = (chunk) => {
    const match = chunk.toString().match(/https:\/\/[a-z0-9-]+\.trycloudflare\.com/i);
    if (!match || announced) return;
    announced = true;
    const url = `${match[0]}/mcp?token=${encodeURIComponent(TOKEN)}`;
    log(`Public MCP URL: ${url}`);
    saveState(url);
  };
  tunnel.stdout.on('data', inspect);
  tunnel.stderr.on('data', inspect);
  tunnel.on('error', (error) => log(`Tunnel unavailable: ${error.message}. Local MCP is still running.`));
}

function shutdown() {
  acp.stop();
  try { tunnel?.kill(); } catch {}
  try { server.close(); } catch {}
}

process.on('SIGINT', () => { shutdown(); process.exit(0); });
process.on('SIGTERM', () => { shutdown(); process.exit(0); });
process.on('exit', shutdown);

server.on('error', (error) => {
  if (error.code === 'EADDRINUSE') {
    log(`Port ${PORT} is already in use; another bridge may already be running.`);
    process.exit(0);
  }
  throw error;
});

server.listen(PORT, HOST, () => {
  const localUrl = `http://127.0.0.1:${PORT}/mcp?token=${encodeURIComponent(TOKEN)}`;
  log(`Local MCP URL: ${localUrl}`);
  saveState(null);
  startTunnel();
});
