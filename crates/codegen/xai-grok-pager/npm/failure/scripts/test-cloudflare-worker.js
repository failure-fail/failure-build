#!/usr/bin/env node
'use strict';

const assert = require('assert');
const { discoverAccounts, deployProxyWorker } = require('../bin/cloudflare-worker');

const calls = [];

global.fetch = async (url, options = {}) => {
  calls.push({ url, options });
  if (url.includes('/accounts?')) {
    return new Response(JSON.stringify({
      success: true,
      result: [{ id: 'account-123', name: 'Failure Test Account' }],
    }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
  }
  if (url.endsWith('/workers/subdomain')) {
    return new Response(JSON.stringify({ success: true, result: { subdomain: 'failure-test' } }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
  }
  return new Response(JSON.stringify({ success: true, result: {} }), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  });
};

async function main() {
  const accounts = await discoverAccounts('test-token');
  assert.deepStrictEqual(accounts, [{ id: 'account-123', name: 'Failure Test Account' }]);
  assert.strictEqual(calls[0].options.headers.authorization, 'Bearer test-token');

  const config = {
    apiToken: 'test-token',
    accountId: 'account-123',
    workerName: 'Failure MCP Test',
  };
  const url = await deployProxyWorker(
    config,
    'https://temporary-origin.trycloudflare.com',
    'access-secret',
  );

  assert.strictEqual(url, 'https://failure-mcp-test.failure-test.workers.dev/mcp?token=access-secret');
  assert.strictEqual(calls.length, 4);

  const upload = calls[1];
  assert.match(upload.url, /accounts\/account-123\/workers\/scripts\/failure-mcp-test$/);
  assert.strictEqual(upload.options.method, 'PUT');
  assert.strictEqual(upload.options.headers.authorization, 'Bearer test-token');
  assert(upload.options.body instanceof FormData);

  const metadata = JSON.parse(upload.options.body.get('metadata'));
  assert.strictEqual(metadata.main_module, 'worker.mjs');
  assert(metadata.bindings.some((binding) => binding.name === 'UPSTREAM_ORIGIN' && binding.text.includes('trycloudflare.com')));
  assert(metadata.bindings.some((binding) => binding.name === 'ACCESS_TOKEN' && binding.text === 'access-secret'));

  const modulePart = upload.options.body.get('worker.mjs');
  const source = await modulePart.text();
  assert.match(source, /env\.UPSTREAM_ORIGIN/);
  assert.match(source, /env\.ACCESS_TOKEN/);
  assert.match(source, /authorization/);

  const enable = calls[2];
  assert.match(enable.url, /failure-mcp-test\/subdomain$/);
  assert.strictEqual(enable.options.method, 'POST');
  assert.deepStrictEqual(JSON.parse(enable.options.body), { enabled: true, previews_enabled: false });

  process.stdout.write('Cloudflare Worker deployment test passed.\n');
}

main().catch((error) => {
  process.stderr.write(`${error.stack || error}\n`);
  process.exitCode = 1;
});
