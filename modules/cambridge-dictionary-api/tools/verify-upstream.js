// Compare extracted parser with verbatim upstream code on saved source pages.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { parseCambridgePage } = require('../dictionary/cambridge');
const { upstreamResult } = require('../tests/upstream-oracle');
(async () => {
  const folder = process.argv[2];
  if (!folder) throw Error('Usage: node tools/verify-upstream.js <saved-page-folder>');
  const records = [];
  for (const file of fs.readdirSync(folder).filter(name => name.endsWith('.html'))) {
    const html = fs.readFileSync(path.join(folder, file), 'utf8');
    const language = file === 'runtime-en.html' ? 'en' : 'en-cn';
    const old = await upstreamResult(html, language, file.replace(/\.html$/, ''));
    let result, error;
    try { const value = parseCambridgePage(html); result = { word: value.word, pos: value.pos, verbs: [], pronunciation: value.pronunciation, definition: value.definition }; }
    catch (failure) { error = failure; }
    if (old.status === 404) { assert.equal(error?.status, 404); records.push({ file, notFound: true, matchesUpstream: true }); continue; }
    assert.equal(old.status, 200);
    assert.deepEqual(JSON.parse(JSON.stringify(result)), old.body);
    records.push({ file, word: result.word, definitions: result.definition.length, examples: result.definition.reduce((n, d) => n + d.example.length, 0), pronunciation: result.pronunciation.length, forms: result.verbs.length, matchesUpstream: true });
  }
  assert.ok(records.length > 0, 'No real pages were checked');
  console.log(JSON.stringify({ source: 'Saved public Cambridge pages, compared to executed original data.js; not current network timing', records }, null, 2));
})().catch(error => { console.error(error); process.exitCode = 1; });
