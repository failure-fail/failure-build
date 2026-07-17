#!/usr/bin/env node
'use strict';

const assert = require('assert');
const fs = require('fs');
const http = require('http');
const os = require('os');
const path = require('path');
const { spawn } = require('child_process');

const packageRoot = path.resolve(__dirname, '..');
const serverScript = path.join(packageRoot, 'bin', 'mcp-server.js');
const port = 34220 + (process.pid % 1000);
const token = 'failure-mcp-test-token';
const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'failure-mcp-test-'));
const fakeFailure = path.join(tempDir, process.platform === 'win32' ? 'fake-failure.cmd' : 'fake-failure');

if (process.platform === 'win32') {
  fs.writeFileSync(fakeFailure, `@echo off\nnode "${path.join(tempDir, 'fake-failure.js')}" %*\n`);
} else {
  fs.writeFileSync(fakeFailure, `#!/usr/bin/env node\nrequire(${JSON.stringify(path.join(tempDir, 'fake-failure.js'))});\n`, { mode: 0o755 });
}

fs.writeFileSync(path.join(tempDir, 'fake-failure.js'), `
'use strict';
const readline = require('readline');
if (process.argv.includes('sessions')) {
  process.stdout.write(JSON.stringify([{ sessionId: 'saved-session', title: 'Saved' }]) + '\\n');
  process.exit(0);
}
const rl = readline.createInterface({ input: process.stdin });
rl.on('line', (line) => {
  const request = JSON.parse(line);
  let result = {};
  if (request.method === 'initialize') {
    result = { protocolVersion: 1, agentInfo: { name: 'fake-failure', version: '1' } };
  } else if (request.method === 'session/new') {
    result = { sessionId: 'new-session' };
  } else if (request.method === 'session/load') {
    result = { sessionId: request.params.sessionId };
  } else if (request.method === 'session/prompt') {
    result = { stopReason: 'end_turn', sessionId: request.params.sessionId };
  }
  process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: request.id, result }) + '\\n');
});
`);

function request(method, urlPath, body, authorized = true) {
  return new Promise((resolve, reject) => {
    const encoded = body === undefined ? null : Buffer.from(JSON.stringify(body));
    const req = http.request({
      host: '127.0.0.1',
      port,
      method,
      path: urlPath,
      headers: {
        ...(authorized ? { authorization: `Bearer ${token}` } : {}),
        ...(encoded ? { 'content-type': 'application/json', 'content-length': encoded.length } : {}),
      },
    }, (res) => {
      const chunks = [];
      res.on('data', (chunk) => chunks.push(chunk));
      res.on('end', () => {
        const text = Buffer.concat(chunks).toString('utf8');
        let json = null;
        try { json = text ? JSON.parse(text) : null; } catch {}
        resolve({ status: res.statusCode, headers: res.headers, text, json });
      });
    });
    req.on('error', reject);
    if (encoded) req.write(encoded);
    req.end();
  });
}

function rpc(id, method, params = {}) {
  return request('POST', '/mcp', { jsonrpc: '2.0', id, method, params });
}

async function waitForHealth() {
  for (let i = 0; i < 80; i += 1) {
    try {
      const response = await request('GET', '/health', undefined, false);
      if (response.status === 200) return;
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error('MCP bridge did not become healthy');
}

async function main() {
  const child = spawn(process.execPath, [serverScript, '--failure-bin', fakeFailure], {
    stdio: ['ignore', 'ignore', 'pipe'],
    env: {
      ...process.env,
      FAILURE_MCP_PORT: String(port),
      FAILURE_MCP_BIND: '127.0.0.1',
      FAILURE_MCP_TOKEN: token,
      FAILURE_MCP_TUNNEL: '0',
      HOME: tempDir,
      USERPROFILE: tempDir,
    },
  });
  let stderr = '';
  child.stderr.on('data', (chunk) => { stderr += chunk.toString(); });

  try {
    await waitForHealth();

    const unauthorized = await request('POST', '/mcp', { jsonrpc: '2.0', id: 1, method: 'ping' }, false);
    assert.strictEqual(unauthorized.status, 401);

    const initialized = await rpc(2, 'initialize', { protocolVersion: '2025-03-26' });
    assert.strictEqual(initialized.status, 200);
    assert.strictEqual(initialized.json.result.serverInfo.name, 'failure-build-remote');

    const listed = await rpc(3, 'tools/list');
    const names = listed.json.result.tools.map((tool) => tool.name);
    assert(names.includes('failure_new_chat'));
    assert(names.includes('failure_continue_chat'));
    assert(names.includes('failure_send_message'));
    assert(names.includes('failure_rpc'));

    const created = await rpc(4, 'tools/call', {
      name: 'failure_new_chat',
      arguments: { cwd: tempDir },
    });
    assert.strictEqual(created.json.result.isError, false);
    assert.match(created.json.result.content[0].text, /new-session/);

    const prompted = await rpc(5, 'tools/call', {
      name: 'failure_send_message',
      arguments: { session_id: 'new-session', message: 'Create a file.' },
    });
    assert.strictEqual(prompted.json.result.isError, false);
    assert.match(prompted.json.result.content[0].text, /end_turn/);

    const sessions = await rpc(6, 'tools/call', {
      name: 'failure_list_sessions',
      arguments: {},
    });
    assert.match(sessions.json.result.content[0].text, /saved-session/);

    const state = JSON.parse(fs.readFileSync(path.join(tempDir, '.failure', 'mcp.json'), 'utf8'));
    assert.strictEqual(state.token, token);
    assert.match(state.localUrl, /342/);

    process.stdout.write('Failure MCP bridge smoke test passed.\n');
  } finally {
    child.kill();
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}

main().catch((error) => {
  process.stderr.write(`${error.stack || error}\n`);
  process.exitCode = 1;
});
