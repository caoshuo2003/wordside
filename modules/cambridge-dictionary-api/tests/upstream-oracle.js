const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const cheerio = require('cheerio');

// Execute the saved upstream data.js verbatim with an offline transport.
// No server is started and no real network call is made by this oracle.
async function upstreamResult(html, language = 'en-cn', entry = 'example') {
  const routes = new Map();
  const requests = [];
  const app = { use() {}, get(route, handler) { routes.set(route, handler); } };
  const sandbox = {
    module: { exports: {} },
    __dirname: path.join(__dirname, 'upstream'),
    console: { error() {}, warn() {} },
    require(name) {
      if (name === 'cheerio') return cheerio;
      if (name === 'express') return () => app;
      if (name === 'cors') return () => () => {};
      if (name === 'axios') return { create: () => ({ get: async (url) => {
        requests.push(url);
        return { status: 200, data: url.startsWith('https://simple.wiktionary.org/') ? '<html></html>' : html };
      } }) };
      throw new Error(`Unexpected upstream dependency: ${name}`);
    },
  };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'upstream/data.js'), 'utf8'), sandbox);
  let status, body;
  await routes.get('/api/dictionary/:language/:entry')({ params: { language, entry } }, {
    status(value) { status = value; return this; },
    json(value) { body = JSON.parse(JSON.stringify(value)); },
  });
  return { status, body, requests };
}
module.exports = { upstreamResult };
