const test = require('node:test');
const assert = require('node:assert/strict');
const { createCambridge, buildSuggestionUrl, parseCambridgeSuggestions } = require('../dictionary/cambridge');
const { createLookup } = require('../lookup');
const { html, landing, suggestionPage } = require('./fixture');
for (const [language, dataset, nation] of [['en', 'english', 'us'], ['uk', 'english', 'uk'], ['en-cn', 'english-chinese-simplified', 'us'], ['en-tw', 'english-chinese-traditional', 'us']]) {
  test(`spellcheck source and candidate order: ${language}`, () => {
    assert.deepEqual(parseCambridgeSuggestions(suggestionPage({ language: dataset, nation }), language, 'termnal'), ['terminal', 'terminals']);
    const url = new URL(buildSuggestionUrl(language, 'two words'));
    assert.equal(url.hostname, 'dictionary.cambridge.org'); assert.equal(url.pathname, `/${nation}/spellcheck/${dataset}/`); assert.equal(url.searchParams.get('q'), 'two words');
    assert.deepEqual(parseCambridgeSuggestions(suggestionPage({ language: dataset, nation, empty: true }), language, 'termnal'), []);
  });
}
test('candidate list excludes the query, duplicates, unsafe words and navigation', () => {
  const words = ['termnal', 'terminal', 'Terminal', 'terminals', 'bad/path', '<img>', 'hello\nworld', ...Array.from({ length: 20 }, (_, i) => 'candidate' + i)];
  const result = parseCambridgeSuggestions(suggestionPage({ words }), 'en-cn', 'termnal');
  assert.equal(result.length, 10); assert.deepEqual(result.slice(0, 2), ['terminal', 'terminals']); assert.ok(!result.includes('navigation')); assert.ok(!result.includes('termnal'));
});
test('only candidate links with matching Cambridge dataset, q and label are accepted', () => {
  const page = suggestionPage({ words: ['terminal'] });
  for (const invalid of [page.replace('href="https://dictionary.cambridge.org/us/search', 'href="https://example.com/us/search'), page.replace('/search/english-chinese-simplified/', '/search/english/'), page.replace('q=terminal', 'q=other'), page.replace('/search/', '/browse/'), page.replace('href="https://dictionary.cambridge.org', 'href="https://user@dictionary.cambridge.org'), page.replace('q=terminal', 'q=terminal&amp;q=other')]) {
    assert.deepEqual(parseCambridgeSuggestions(invalid, 'en-cn', 'termnal'), []);
  }
});
test('truncated, challenged, mismatched and structurally changed suggestions never become candidates', () => {
  const page = suggestionPage();
  for (const invalid of [page.slice(0, -7), page.replace('value="termnal"', 'value="other"'), page.replace('content="https://dictionary.cambridge.org', 'content="https://example.com'), page.replace('content="https://dictionary.cambridge.org', 'content="https://user@dictionary.cambridge.org'), page.replace('<span class="tb">termnal', '<span class="tb">other'), page.replace('class="hul-u"', 'class="changed"'), page.replace('Search suggestions for', 'Changed heading')]) {
    assert.throws(() => parseCambridgeSuggestions(invalid, 'en-cn', 'termnal'), { status: 502, code: 'INVALID_CAMBRIDGE_PAGE' });
  }
  assert.throws(() => parseCambridgeSuggestions(page.replace('<body>', '<body><form id="challenge-form"></form>'), 'en-cn', 'termnal'), { status: 502, code: 'CAMBRIDGE_BLOCKED' });
});
test('confirmed missing returns 404 and candidates, never a substituted successful entry', async () => {
  const requests = [];
  const provider = createCambridge({ client: { get: async (url, options) => { requests.push(url); assert.ok(options.signal instanceof AbortSignal); return { status: 200, data: url.includes('/spellcheck/') ? suggestionPage() : landing }; } } });
  await assert.rejects(provider.lookup('en-cn', 'termnal'), error => error.status === 404 && error.code === 'WORD_NOT_FOUND' && assert.deepEqual(error.suggestions, ['terminal', 'terminals']) === undefined);
  assert.equal(requests.length, 2); assert.equal(requests[1], buildSuggestionUrl('en-cn', 'termnal'));
});
test('normal hits and primary network/challenge/incomplete errors do not request suggestions', async () => {
  for (const response of [{ status: 200, data: html }, { status: 503, data: '' }, { status: 200, data: '<html></html>' }, { status: 200, data: html.replace('<title>', '<title>Access denied ') }]) {
    let count = 0; const provider = createCambridge({ client: { get: async () => { count++; return response; } } });
    try { await provider.lookup('en-cn', 'example'); } catch (error) { assert.equal(error.status, 502); assert.equal(error.suggestions, undefined); }
    assert.equal(count, 1);
  }
  for (const cause of [{ code: 'ETIMEDOUT' }, { response: { status: 403 } }, { code: 'ENOTFOUND' }]) {
    let count = 0; const provider = createCambridge({ client: { get: async () => { count++; throw cause; } } });
    await assert.rejects(provider.lookup('en-cn', 'example'), error => error.status !== 404 && error.suggestions === undefined); assert.equal(count, 1);
  }
});
test('unavailable, empty or rejected optional suggestion pages preserve the confirmed 404', async () => {
  for (const optional of [{ status: 503 }, { status: 200, data: suggestionPage({ empty: true }) }, { status: 200, data: '<html></html>' }, { status: 200, data: suggestionPage().replace('<title>', '<title>Just a moment ') }, { code: 'ERR_CANCELED' }]) {
    let count = 0; const provider = createCambridge({ client: { get: async () => { if (++count === 1) return { status: 200, data: landing }; if (optional.code) throw optional; return optional; } } });
    await assert.rejects(provider.lookup('en-cn', 'termnal'), error => error.status === 404 && error.code === 'WORD_NOT_FOUND' && error.suggestions === undefined); assert.equal(count, 2);
  }
});
test('suggestion deadline uses the remaining original request budget', async t => {
  let clockReads = 0, calls = 0;
  t.mock.method(Date, 'now', () => ++clockReads === 1 ? 0 : 9800);
  const provider = createCambridge({ client: { get: async (_, options) => {
    if (++calls === 1) return { status: 200, data: landing };
    assert.equal(options.signal.aborted, false);
    await new Promise(resolve => setTimeout(resolve, 250)); assert.equal(options.signal.aborted, true); throw { code: 'ERR_CANCELED' };
  } } });
  await assert.rejects(provider.lookup('en-cn', 'termnal'), { status: 404, code: 'WORD_NOT_FOUND' }); assert.equal(calls, 2);
});
test('simultaneous misses share the suggestion request and failures are never cached', async () => {
  let count = 0;
  const lookup = createLookup({ cambridge: createCambridge({ client: { get: async url => { count++; await new Promise(resolve => setImmediate(resolve)); return { status: 200, data: url.includes('/spellcheck/') ? suggestionPage() : landing }; } } }) });
  const results = await Promise.allSettled([lookup('en-cn', 'termnal'), lookup('en-cn', 'termnal')]);
  assert.ok(results.every(result => result.status === 'rejected' && result.reason.suggestions[0] === 'terminal')); assert.equal(count, 2);
  await assert.rejects(lookup('en-cn', 'termnal'), { status: 404 }); assert.equal(count, 4); lookup.close();
});
