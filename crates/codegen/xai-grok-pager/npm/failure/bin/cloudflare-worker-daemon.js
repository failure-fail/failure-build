#!/usr/bin/env node
'use strict';

const fs = require('fs');
const os = require('os');
const path = require('path');
const { readConfig, deployProxyWorker } = require('./cloudflare-worker');

const STATE_PATH = path.join(os.homedir(), '.failure', 'mcp.json');
const POLL_MS = 500;
let stopped = false;
let lastOrigin = null;
let deploying = false;

function log(message) {
  process.stderr.write(`[failure-mcp-worker] ${message}\n`);
}

function readState() {
  try { return JSON.parse(fs.readFileSync(STATE_PATH, 'utf8')); } catch { return null; }
}

function writeState(state) {
  const temporary = `${STATE_PATH}.tmp.${process.pid}`;
  fs.writeFileSync(temporary, JSON.stringify(state, null, 2), { mode: 0o600 });
  fs.renameSync(temporary, STATE_PATH);
}

function upstreamOrigin(publicUrl) {
  if (!publicUrl) return null;
  try {
    const url = new URL(publicUrl);
    if (!url.hostname.endsWith('.trycloudflare.com')) return null;
    return url.origin;
  } catch {
    return null;
  }
}

async function tick() {
  if (stopped || deploying) return;
  const config = readConfig();
  if (!config?.enabled) return;
  const state = readState();
  const origin = upstreamOrigin(state?.publicUrl);
  if (!origin || origin === lastOrigin) return;

  deploying = true;
  try {
    const workerUrl = await deployProxyWorker(config, origin, state.token);
    const latest = readState() || state;
    latest.workerUrl = workerUrl;
    latest.workerName = config.workerName || 'failure-mcp';
    latest.workerUpdatedAt = new Date().toISOString();
    writeState(latest);
    lastOrigin = origin;
    log(`Stable Worker MCP URL: ${workerUrl}`);
  } catch (error) {
    log(`Worker update failed: ${error.message}`);
  } finally {
    deploying = false;
  }
}

const interval = setInterval(() => {
  tick().catch((error) => log(error.message));
}, POLL_MS);
interval.unref?.();

function shutdown() {
  stopped = true;
  clearInterval(interval);
}

process.on('SIGINT', () => { shutdown(); process.exit(0); });
process.on('SIGTERM', () => { shutdown(); process.exit(0); });
process.on('exit', shutdown);

tick().catch((error) => log(error.message));
