// Live local-module probe. It starts no HTTP server and opens no GUI.
const fs = require('node:fs');
const path = require('node:path');
const { performance } = require('node:perf_hooks');
const { createHttpClient } = require('../lib/http-client');
const { createCambridge } = require('../dictionary/cambridge');
const { createLookup } = require('../lookup');
const client = createHttpClient();
const requests = [];
const redirects = [];
const restrictRedirect = client.defaults.beforeRedirect;
client.defaults.beforeRedirect = (options) => { restrictRedirect(options); redirects.push(options.href); };
client.interceptors.request.use(config => { requests.push(config.url); return config; });
const lookup = createLookup({ cambridge: createCambridge({ client }) });
(async () => {
  const cases = [ ['en-cn', 'terminal'], ['en-cn', 'terminal'], ['en-cn', 'run'], ['en-cn', 'CPU'], ['en', 'runtime'], ['en', 'runtime'], ['en-cn', 'run on the spot'], ['en-cn', 'wordsidezzxinvalid'] ];
  const records = [];
  for (const [language, word] of cases) {
    const before = requests.length; const redirectBefore = redirects.length; const start = performance.now();
    let result, error;
    try { result = await lookup(language, word); }
    catch (failure) { error = failure; }
    records.push({ language, query: word, milliseconds: Number((performance.now() - start).toFixed(2)), axiosGetCalls: requests.length - before, redirects: redirects.length - redirectBefore, word: result?.word, definitions: result?.definition.length, examples: result?.definition.reduce((n, d) => n + d.example.length, 0), forms: result?.verbs.length, status: error?.status || 200, code: error?.code });
  }
  const output = { mode: 'Windows Node, real Cambridge via local extracted module; process-local cache enabled; no public dictionary API or Wiktionary', requestObservation: 'Axios GET invocations and successful same-site redirects; cache hits make zero outbound calls', records, destinations: requests, redirectDestinations: redirects };
  if (process.argv[2]) { fs.mkdirSync(path.dirname(process.argv[2]), { recursive: true }); fs.writeFileSync(process.argv[2], JSON.stringify(output, null, 2) + '\n'); }
  console.log(JSON.stringify(output, null, 2));
  const expected = [200, 200, 200, 200, 200, 200, 200, 404];
  if (records.some((r, i) => r.status !== expected[i]) || [...requests, ...redirects].some(url => new URL(url).hostname !== 'dictionary.cambridge.org')) process.exitCode = 1;
})().catch(error => { console.error(error.name); process.exitCode = 1; }).finally(() => lookup.close());
