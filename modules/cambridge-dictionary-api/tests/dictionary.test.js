const test = require('node:test');
const assert = require('node:assert/strict');
const { parseCambridgePage, createCambridge, buildDictionaryUrl } = require('../dictionary/cambridge');
const { createLookup } = require('../lookup');
const { MemoryCache } = require('../lib/cache');
const { html, landing, phraseHtml, phraseEntry } = require('./fixture');
const { upstreamResult } = require('./upstream-oracle');

test('package entry exports the local lookup module without starting a service', () => {
  assert.equal(require('..').createLookup, createLookup);
});

for (const language of ['en', 'uk', 'en-cn', 'en-tw']) {
  test(`unchanged upstream JSON contract: ${language}`, async () => {
    const old = await upstreamResult(html, language);
    const requests = [];
    const cambridge = createCambridge({ client: { get: async (url) => { requests.push(url); return { status: 200, data: html }; } } });
    const lookup = createLookup({ cambridge });
    const result = JSON.parse(JSON.stringify(await lookup(language, 'example')));
    assert.equal(old.status, 200);
    assert.deepEqual(result, old.body);
    assert.equal(requests.length, 1);
    assert.equal(requests[0], old.requests[0]);
    assert.equal(old.requests.length, 2); // Old path also queried Wiktionary.
    assert.match(old.requests[1], /^https:\/\/simple\.wiktionary\.org\//);
    assert.equal(new URL(requests[0]).hostname, 'dictionary.cambridge.org');
    lookup.close();
  });
}
test('all core translations/examples retained, missing translations remain empty', () => {
  const result = parseCambridgePage(html);
  assert.equal(result.definition.length, 2);
  assert.equal(result.definition[0].translation, '测试释义；补充译文');
  assert.equal(result.definition[0].example.length, 2);
  assert.equal(result.definition[0].example[1].translation, '');
  assert.equal(result.definition[1].translation, '');
  assert.equal(result.pronunciation.length, 2);
  assert.equal(result.definition[0].source, 'SYNTHETIC');
  assert.ok(!JSON.stringify(result).includes('Additional unassigned'));
});
test('entry and language cannot change the fixed Cambridge destination', () => {
  assert.equal(buildDictionaryUrl('en-cn', 'run on the spot'), 'https://dictionary.cambridge.org/us/dictionary/english-chinese-simplified/run%20on%20the%20spot');
  for (const language of ['english', 'constructor', '__proto__', 'toString']) assert.throws(() => buildDictionaryUrl(language, 'word'), { status: 400 });
  for (const word of ['', ' ', '..', '/', 'one\ntwo', '\\host', 'a'.repeat(121)]) assert.throws(() => buildDictionaryUrl('en-cn', word), { status: 400 });
  assert.match(buildDictionaryUrl('en-cn', 'https:example.com?x=y'), /https%3Aexample\.com%3Fx%3Dy$/);
});
test('incomplete HTML, definitions and examples never become successful entries', () => {
  for (const source of [html.slice(0, -7), html.replace('class="def ddef_d db"', 'class="broken"'), html.replace('class="def-body ddef_b"', 'class="broken"'), html.replace('class="eg deg"', 'class="broken"')]) {
    assert.throws(() => parseCambridgePage(source), { status: 502, code: 'INVALID_CAMBRIDGE_PAGE' });
  }
});
test('verified dictionary landing means missing; arbitrary HTML and challenges do not', () => {
  assert.throws(() => parseCambridgePage(landing), { status: 404, code: 'WORD_NOT_FOUND' });
  assert.throws(() => parseCambridgePage('<html><body>Unexpected response</body></html>'), { status: 502 });
  assert.throws(() => parseCambridgePage(landing.replace('Cambridge dictionary', 'Just a moment')), { status: 502, code: 'CAMBRIDGE_BLOCKED' });
  assert.throws(() => parseCambridgePage(html.replace('<body>', '<body><form id="challenge-form"></form>')), { status: 502, code: 'CAMBRIDGE_BLOCKED' });
});
for (const [cause, status, code] of [
  [{ response: { status: 404 } }, 404, 'WORD_NOT_FOUND'],
  [{ response: { status: 403 } }, 502, 'CAMBRIDGE_BLOCKED'],
  [{ response: { status: 429 } }, 502, 'CAMBRIDGE_BLOCKED'],
  [{ response: { status: 503 } }, 502, 'CAMBRIDGE_UNAVAILABLE'],
  [{ code: 'ECONNABORTED' }, 504, 'CAMBRIDGE_TIMEOUT'],
  [{ code: 'ERR_CANCELED' }, 504, 'CAMBRIDGE_TIMEOUT'],
  [{ code: 'ENOTFOUND' }, 502, 'CAMBRIDGE_UNAVAILABLE'],
]) {
  test(`network failure has its own result: ${code}/${cause.response?.status || cause.code}`, async () => {
    const provider = createCambridge({ client: { get: async () => { throw cause; } } });
    await assert.rejects(provider.lookup('en-cn', 'example'), { status, code });
  });
}
test('full lookup deduplicates in-flight requests and never returns before full data', async () => {
  let finish, count = 0;
  const lookup = createLookup({ cambridge: { lookup() { count++; return new Promise(resolve => { finish = resolve; }); } } });
  let completed = false;
  const a = lookup('en-cn', 'example').then(value => { completed = true; return value; });
  const b = lookup('en-cn', 'example');
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(count, 1); assert.equal(completed, false);
  finish(parseCambridgePage(html));
  const [one, two] = await Promise.all([a, b]);
  one.definition[0].text = 'mutated';
  assert.notEqual(two.definition[0].text, 'mutated');
  const cached = await lookup('en-cn', 'example');
  assert.equal(count, 1); assert.notEqual(cached.definition[0].text, 'mutated');
  lookup.close();
});
test('failed responses are not cached and subsequent lookup recovers', async () => {
  let count = 0;
  const lookup = createLookup({ cambridge: { async lookup() { if (++count === 1) throw Error('first request failed'); return parseCambridgePage(html); } } });
  await assert.rejects(lookup('en-cn', 'example'), /first request failed/);
  assert.equal((await lookup('en-cn', 'example')).word, 'example');
  assert.equal(count, 2); lookup.close();
});
test('cache uses exact URL keys and bounded, expiring JSON entries', async () => {
  let now = 0;
  const cache = new MemoryCache({ maxEntries: 2, maxBytes: 100, ttlMs: 10, now: () => now });
  cache.set('a', { a: 1 }); cache.set('b', { b: 2 }); cache.get('a'); cache.set('c', { c: 3 });
  assert.equal(cache.entries.size, 2); assert.equal(cache.get('b'), undefined);
  assert.deepEqual(cache.get('a'), { a: 1 });
  cache.set('big', { text: 'x'.repeat(101) }); assert.equal(cache.get('big'), undefined);
  now = 10; assert.equal(cache.get('a'), undefined);
  assert.ok(cache.bytes <= 100); cache.clear(); assert.equal(cache.bytes, 0);
  let count = 0;
  const lookup = createLookup({ cambridge: { async lookup() { count++; return parseCambridgePage(html); } } });
  for (const word of ['foo-bar', 'foo bar', 'foo_bar']) await lookup('en-cn', word);
  assert.equal(count, 3); lookup.close();
});

test('complete bare English definitions are retained without losing translations or examples', async () => {
  const bare = '<div class="def-block ddef_block"><div class="ddef_h"><div class="def ddef_d db">A synthetic meaning without examples.</div></div></div>';
  const page = html.replace('</div></div></div><div class="daccord">', bare + '</div></div></div><div class="daccord">');
  const old = await upstreamResult(page, 'en');
  const lookup = createLookup({ cambridge: createCambridge({ client: { get: async () => ({ status: 200, data: page }) } }) });
  try {
    const entry = await lookup('en', 'example'); assert.deepEqual(entry, old.body); assert.equal(entry.definition.length, 3); assert.deepEqual(entry.definition[2].example, []); assert.equal(entry.definition[2].translation, '');
    for (const content of ['<div class="examp dexamp"><span class="eg deg">Moved example.</span></div>', '<span class="trans dtrans">Moved translation.</span>']) {
      assert.throws(() => parseCambridgePage(page.replace(bare, bare.replace('</div></div>', content + '</div></div>'))), { status: 502, code: 'INVALID_CAMBRIDGE_PAGE' });
    }
    assert.throws(() => parseCambridgePage(page.replace('</body>', '<div class="def-body ddef_b"></div></body>')), { status: 502, code: 'INVALID_CAMBRIDGE_PAGE' });
    assert.throws(() => parseCambridgePage(page.replace('<div class="def-block ddef_block">', '<div class="def-block ddef_block"><span class="trans dtrans">Orphan translation.</span>')), { status: 502, code: 'INVALID_CAMBRIDGE_PAGE' });
    assert.throws(() => parseCambridgePage(page.replace(bare, bare.replace('</div></div>', '<div class="def-body ddef_b"></div><div class="def-body ddef_b"></div></div></div>'))), { status: 502, code: 'INVALID_CAMBRIDGE_PAGE' });
    assert.throws(() => parseCambridgePage(page.slice(0, -7)), { status: 502, code: 'INVALID_CAMBRIDGE_PAGE' });
  } finally { lookup.close(); }
});


test('standalone phrase pages retain their heading, grammatical label, translations and all examples', () => {
  assert.deepEqual(JSON.parse(JSON.stringify(parseCambridgePage(phraseHtml))), phraseEntry);
  // A normal headword keeps priority on a mixed dictionary page.
  const mixed = html.replace('</body>', phraseHtml.match(/<body>([\s\S]*)<\/body>/)[1] + '</body>');
  const entry = JSON.parse(JSON.stringify(parseCambridgePage(mixed)));
  assert.equal(entry.word, 'example');
  assert.equal(entry.definition.length, 3);
  assert.deepEqual(entry.definition[2], { ...phraseEntry.definition[0], id: 2 });
});
test('phrase headings do not excuse missing or unassigned definition content', () => {
  for (const page of [phraseHtml.slice(0, -7), phraseHtml.replace('class="def ddef_d db"', 'class="broken"'),
    phraseHtml.replace('class="eg deg"', 'class="broken"'), phraseHtml.replace('class="phrase-di-block dphrase-di-block"', 'class="broken"'),
    phraseHtml.replace('class="di-title"', 'class="broken"')]) {
    assert.throws(() => parseCambridgePage(page), { status: 502, code: 'INVALID_CAMBRIDGE_PAGE' });
  }
});
