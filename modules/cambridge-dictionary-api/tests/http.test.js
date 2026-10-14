const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const { once } = require('node:events');
const { gzipSync } = require('node:zlib');
const { createApp } = require('../data');
const { createLookup } = require('../lookup');
const { createCambridge } = require('../dictionary/cambridge');
const { createHttpClient } = require('../lib/http-client');
const { html } = require('./fixture');

async function withServer(handler, run) {
  const server = http.createServer(handler);
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  const url = `http://127.0.0.1:${server.address().port}`;
  try { await run(url, server); }
  finally { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); }
}
test('original HTTP route and UI work with the extracted local module', async () => {
  const requested = [];
  const cambridge = createCambridge({ client: { get: async (url) => { requested.push(url); return { status: 200, data: html }; } } });
  const lookup = createLookup({ cambridge }); const app = createApp({ lookup });
  try { await withServer(app, async url => {
    const response = await fetch(url + '/api/dictionary/en-cn/example');
    assert.equal(response.status, 200); const body = await response.json();
    assert.deepEqual(Object.keys(body), ['word', 'pos', 'verbs', 'pronunciation', 'definition']);
    assert.equal(body.word, 'example'); assert.equal(body.definition.length, 2); assert.deepEqual(body.verbs, []);
    assert.equal(response.headers.get('access-control-allow-origin'), '*');
    assert.equal((await fetch(url + '/api/dictionary/en-cn/example')).status, 200);
    assert.equal(requested.length, 1);
    assert.equal((await fetch(url + '/api/dictionary/english/example')).status, 400);
    assert.equal((await fetch(url + '/api/dictionary/en-cn/%E0%A4%A')).status, 400);
    assert.equal((await fetch(url + '/api/dictionary/en-cn/%2F')).status, 400);
    assert.equal((await fetch(url + '/api/dictionary/en-cn/run%20on%20the%20spot')).status, 200);
    assert.match(requested.at(-1), /run%20on%20the%20spot$/);
    assert.deepEqual(await (await fetch(url + '/healthz')).json(), { status: 'ok' });
    const root = await fetch(url + '/'); assert.equal(root.status, 200); assert.match(await root.text(), /Cambridge/);
  }); } finally { app.locals.close(); }
});
test('HTTP failure/status distinction survives route extraction', async () => {
  let cause;
  const cambridge = createCambridge({ client: { get: async () => { throw cause; } } });
  const app = createApp({ lookup: createLookup({ cambridge }) });
  try { await withServer(app, async url => {
    for (const [error, status, code] of [[{ response: { status: 404 } }, 404, 'WORD_NOT_FOUND'], [{ response: { status: 503 } }, 502, 'CAMBRIDGE_UNAVAILABLE'], [{ code: 'ECONNABORTED' }, 504, 'CAMBRIDGE_TIMEOUT']]) {
      cause = error; const response = await fetch(url + '/api/dictionary/en-cn/example');
      assert.equal(response.status, status); assert.equal((await response.json()).code, code);
    }
  }); } finally { app.locals.close(); }
});
test('shared original axios client reuses a connection for sequential reads', async () => {
  const client = createHttpClient();
  try { await withServer((req, res) => res.end(html), async (url, server) => {
    let connections = 0; server.on('connection', () => connections++);
    assert.equal((await client.get(url, { proxy: false })).data, html);
    assert.equal((await client.get(url, { proxy: false })).data, html);
    assert.equal(connections, 1);
    client.close();
  }); } finally { client.close(); }
});
test('transport timeout and decoded response size are bounded', async () => {
  const client = createHttpClient();
  try {
    await withServer((req, res) => setTimeout(() => res.end('delayed'), 150), async url => {
      await assert.rejects(client.get(url, { timeout: 20, proxy: false }), { code: 'ECONNABORTED' });
    });
    await withServer((req, res) => res.end('x'.repeat(100)), async url => {
      await assert.rejects(client.get(url, { maxContentLength: 32, proxy: false }), /maxContentLength/);
    });
    // The compressed body is smaller than the cap; the decoded body exceeds it.
    const compressed = gzipSync('x'.repeat(100)); assert.ok(compressed.length < 32);
    await withServer((req, res) => { res.setHeader('Content-Encoding', 'gzip'); res.end(compressed); }, async url => {
      await assert.rejects(client.get(url, { maxContentLength: 32, proxy: false }), /maxContentLength/);
      assert.equal((await client.get(url, { maxContentLength: 128, proxy: false })).data, 'x'.repeat(100));
    });
  } finally { client.close(); }
});
test('redirect restriction respects the destination when a proxy is used', () => {
  const client = createHttpClient();
  try {
    client.defaults.beforeRedirect({ protocol: 'http:', hostname: '127.0.0.1', path: 'https://dictionary.cambridge.org/dictionary/english/run-time', href: 'https://dictionary.cambridge.org/dictionary/english/run-time' });
    assert.throws(() => client.defaults.beforeRedirect({ protocol: 'https:', hostname: 'dictionary.cambridge.org', href: 'https://example.com/a' }), /Unexpected/);
    assert.throws(() => client.defaults.beforeRedirect({ protocol: 'http:', hostname: 'dictionary.cambridge.org', href: 'http://dictionary.cambridge.org/a' }), /Unexpected/);
  } finally { client.close(); }
});
test('foreign redirect is rejected before its target is requested', async () => {
  const client = createHttpClient();
  try { await withServer((req, res) => { res.writeHead(302, { Location: 'https://example.com/unwanted' }); res.end(); }, async url => {
    await assert.rejects(client.get(url, { proxy: false }), /Unexpected Cambridge redirect/);
  }); } finally { client.close(); }
});

test('HTTP 404 exposes Cambridge suggestions and non-missing responses omit them', async () => {
  const { DictionaryError } = require('../lib/errors');
  const lookup = async (_, word) => { const error = new DictionaryError(word === 'missing' ? 404 : 502, word === 'missing' ? 'WORD_NOT_FOUND' : 'CAMBRIDGE_UNAVAILABLE', 'synthetic'); error.suggestions = ['terminal']; throw error; };
  lookup.close = () => {};
  const app = createApp({ lookup });
  try { await withServer(app, async url => {
    const response = await fetch(url + '/api/dictionary/en-cn/missing'); assert.equal(response.status, 404);
    assert.deepEqual(await response.json(), { error: 'synthetic', code: 'WORD_NOT_FOUND', suggestions: ['terminal'] });
    const failure = await fetch(url + '/api/dictionary/en-cn/failure'); assert.equal(failure.status, 502); assert.equal((await failure.json()).suggestions, undefined);
  }); } finally { app.locals.close(); }
});
