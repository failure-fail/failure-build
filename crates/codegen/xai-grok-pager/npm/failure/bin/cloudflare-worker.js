#!/usr/bin/env node
'use strict';

const fs = require('fs');
const os = require('os');
const path = require('path');
const readline = require('readline');

const CONFIG_PATH = path.join(os.homedir(), '.failure', 'cloudflare-worker.json');
const API_BASE = 'https://api.cloudflare.com/client/v4';

function readConfig() {
  try { return JSON.parse(fs.readFileSync(CONFIG_PATH, 'utf8')); } catch { return null; }
}

function writeConfig(config) {
  fs.mkdirSync(path.dirname(CONFIG_PATH), { recursive: true });
  fs.writeFileSync(CONFIG_PATH, JSON.stringify(config, null, 2), { mode: 0o600 });
}

function removeConfig() {
  try { fs.unlinkSync(CONFIG_PATH); } catch {}
}

function ask(rl, prompt, fallback = '') {
  return new Promise((resolve) => rl.question(`${prompt}${fallback ? ` [${fallback}]` : ''}: `, (answer) => resolve(answer.trim() || fallback)));
}

async function apiRequest(config, pathname, options = {}) {
  const response = await fetch(`${API_BASE}${pathname}`, {
    ...options,
    headers: {
      authorization: `Bearer ${config.apiToken}`,
      ...(options.headers || {}),
    },
  });
  const text = await response.text();
  let body;
  try { body = text ? JSON.parse(text) : null; } catch { body = text; }
  if (!response.ok || (body && body.success === false)) {
    const detail = body?.errors?.map((error) => error.message).join('; ') || text || response.statusText;
    throw new Error(`Cloudflare API ${response.status}: ${detail}`);
  }
  return body?.result ?? body;
}

async function discoverAccounts(apiToken) {
  const result = await apiRequest({ apiToken }, '/accounts?per_page=50');
  return Array.isArray(result) ? result : [];
}

async function chooseAccount(rl, apiToken, existingAccountId) {
  if (process.env.CLOUDFLARE_ACCOUNT_ID) return process.env.CLOUDFLARE_ACCOUNT_ID;

  const accounts = await discoverAccounts(apiToken);
  if (!accounts.length) {
    if (existingAccountId) return existingAccountId;
    throw new Error('The Cloudflare token cannot access any accounts. Check its account scope and permissions.');
  }
  if (accounts.length === 1) return accounts[0].id;

  process.stdout.write('Cloudflare accounts available to this token:\n');
  accounts.forEach((account, index) => {
    process.stdout.write(`  ${index + 1}. ${account.name || account.id} (${account.id})\n`);
  });
  const existingIndex = Math.max(0, accounts.findIndex((account) => account.id === existingAccountId));
  const answer = await ask(rl, 'Choose account number', String(existingIndex + 1));
  const index = Number(answer) - 1;
  if (!Number.isInteger(index) || !accounts[index]) throw new Error('Invalid Cloudflare account selection.');
  return accounts[index].id;
}

async function validateConfig(config) {
  if (!config?.apiToken || !config?.accountId) throw new Error('Cloudflare API token and account ID are required.');
  const result = await apiRequest(config, `/accounts/${encodeURIComponent(config.accountId)}/workers/subdomain`);
  return result.subdomain;
}

function workerSource() {
  return `export default {
  async fetch(request, env) {
    const incoming = new URL(request.url);
    const bearer = request.headers.get('authorization');
    const supplied = incoming.searchParams.get('token');
    if (supplied !== env.ACCESS_TOKEN && bearer !== 'Bearer ' + env.ACCESS_TOKEN) {
      return Response.json({ error: 'Missing or invalid Failure MCP token' }, { status: 401 });
    }

    const upstreamBase = new URL(env.UPSTREAM_ORIGIN);
    const upstream = new URL(incoming.pathname + incoming.search, upstreamBase);
    upstream.searchParams.delete('token');

    const headers = new Headers(request.headers);
    headers.set('authorization', 'Bearer ' + env.ACCESS_TOKEN);
    headers.delete('host');

    return fetch(upstream, {
      method: request.method,
      headers,
      body: request.method === 'GET' || request.method === 'HEAD' ? undefined : request.body,
      redirect: 'manual',
    });
  },
};`;
}

async function deployProxyWorker(config, upstreamOrigin, accessToken) {
  const workerName = (config.workerName || 'failure-mcp').toLowerCase().replace(/[^a-z0-9-_]/g, '-');
  const form = new FormData();
  form.set('metadata', JSON.stringify({
    main_module: 'worker.mjs',
    compatibility_date: '2026-07-17',
    bindings: [
      { type: 'plain_text', name: 'UPSTREAM_ORIGIN', text: upstreamOrigin },
      { type: 'plain_text', name: 'ACCESS_TOKEN', text: accessToken },
    ],
  }));
  form.set('worker.mjs', new Blob([workerSource()], { type: 'application/javascript+module' }), 'worker.mjs');

  await apiRequest(config, `/accounts/${encodeURIComponent(config.accountId)}/workers/scripts/${encodeURIComponent(workerName)}`, {
    method: 'PUT',
    body: form,
  });
  await apiRequest(config, `/accounts/${encodeURIComponent(config.accountId)}/workers/scripts/${encodeURIComponent(workerName)}/subdomain`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ enabled: true, previews_enabled: false }),
  });
  const subdomain = await validateConfig(config);
  return `https://${workerName}.${subdomain}.workers.dev/mcp?token=${encodeURIComponent(accessToken)}`;
}

async function configureInteractive() {
  const existing = readConfig() || {};
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  try {
    const apiToken = process.env.CLOUDFLARE_API_TOKEN || await ask(rl, 'Cloudflare API token', existing.apiToken || '');
    if (!apiToken) throw new Error('Cloudflare API token is required.');
    const accountId = await chooseAccount(rl, apiToken, existing.accountId);
    const workerName = await ask(rl, 'Worker name', existing.workerName || 'failure-mcp');
    const config = { apiToken, accountId, workerName, enabled: true };
    const subdomain = await validateConfig(config);
    writeConfig(config);
    process.stdout.write(`Cloudflare Worker access configured.\nWorker target: https://${workerName}.${subdomain}.workers.dev\n`);
  } finally {
    rl.close();
  }
}

async function cli() {
  const command = process.argv[2] || 'status';
  if (command === 'configure') return configureInteractive();
  if (command === 'disable') {
    removeConfig();
    process.stdout.write('Cloudflare Worker access disabled and local credentials removed.\n');
    return;
  }
  if (command === 'status') {
    const config = readConfig();
    if (!config) {
      process.stdout.write('Cloudflare Worker access is not configured.\n');
      return;
    }
    const masked = config.apiToken ? `${config.apiToken.slice(0, 4)}...${config.apiToken.slice(-4)}` : '(missing)';
    process.stdout.write(JSON.stringify({ ...config, apiToken: masked, configPath: CONFIG_PATH }, null, 2) + '\n');
    return;
  }
  throw new Error(`Unknown mcp-worker command: ${command}`);
}

if (require.main === module) {
  cli().catch((error) => {
    process.stderr.write(`failure mcp-worker: ${error.message}\n`);
    process.exitCode = 1;
  });
}

module.exports = {
  CONFIG_PATH,
  readConfig,
  writeConfig,
  discoverAccounts,
  validateConfig,
  deployProxyWorker,
};
