const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const ts = require('typescript');
const source = fs.readFileSync(require.resolve('../dictionary.ts'), 'utf8');
const context = { URL, Error };
vm.runInNewContext(ts.transpileModule(source + '\nglobalThis.schema={readApiEntry,readSuggestions,combineSuggestions,ApiLookupError,normalizeSelection};', { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText, context);
const { readApiEntry, readSuggestions, combineSuggestions, ApiLookupError, normalizeSelection } = context.schema;
const plain = value => JSON.parse(JSON.stringify(value));
const missing = suggestions => ({ entry: null, apiError: '词典服务返回 HTTP 404；未找到此词条', apiStatus: 404, apiCode: 'WORD_NOT_FOUND', suggestions });
test('typed missing error retains candidates and structured status takes precedence over legacy text', () => {
  assert.throws(() => readApiEntry(missing(['terminal', 'terminals']), 'termnal'), error => error instanceof ApiLookupError && error.notFound && assert.deepEqual(plain(error.suggestions), ['terminal', 'terminals']) === undefined);
  for (const payload of [{ ...missing(['terminal']), apiStatus: 502 }, { ...missing(['terminal']), apiCode: 'OTHER_ERROR' }]) {
    assert.throws(() => readApiEntry(payload, 'termnal'), error => !error.notFound && error.suggestions.length === 0);
  }
  assert.throws(() => readApiEntry({ entry: null, apiError: '词典服务返回 HTTP 404；未找到此词条' }, 'termnal'), error => error.notFound && error.suggestions.length === 0);
});
test('legacy error text cannot override partial or conflicting structured metadata', () => {
  for (const payload of [
    {entry:{error:'word not found',suggestions:['terminal']},apiStatus:502,apiCode:'CAMBRIDGE_BLOCKED'},
    {entry:null,apiError:'词典服务返回 HTTP 404',apiCode:'CAMBRIDGE_BLOCKED',suggestions:['terminal']},
    {entry:{error:'word not found',code:'CAMBRIDGE_BLOCKED',suggestions:['terminal']}},
  ]) assert.throws(() => readApiEntry(payload,'termnal'), error => !error.notFound && error.suggestions.length===0);
  assert.throws(() => readApiEntry({entry:{error:'word not found',suggestions:['terminal']}},'termnal'), error => error.notFound && error.suggestions.length===0);
  assert.throws(() => readApiEntry({entry:null,apiError:'词典服务返回 HTTP 404',suggestions:['terminal']},'termnal'), error => error.notFound && error.suggestions.length===0);
});
test('candidate validation rejects malformed, oversized and non-dictionary words', () => {
  for (const value of [{}, ['bad/path'], [null], ['<img>'], ['two\nlines'], [' terminal'], ['a'.repeat(81)], ['one two three four five six'], Array(11).fill('terminal')]) {
    assert.throws(() => readSuggestions(value, 'termnal'), error => error instanceof ApiLookupError && !error.notFound);
  }
  assert.deepEqual(plain(readSuggestions(undefined, 'termnal')), []);
});
test('two dictionary candidate lists preserve order while excluding the query and duplicates', () => {
  assert.deepEqual(plain(combineSuggestions('termnal', ['termnal', 'terminal', 'terminals'], ['Terminal', 'eternal'])), ['terminal', 'terminals', 'eternal']);
  assert.equal(combineSuggestions('query', Array.from({ length: 20 }, (_, i) => 'candidate' + i)).length, 10);
});
test('successful original entries ignore suggestion fields and keep complete definitions', () => {
  const entry = { word: 'terminal', pos: ['noun'], verbs: [], pronunciation: [], definition: [{ text: 'Synthetic meaning', translation: '合成释义', pos: 'noun', source: 'TEST', example: [{ text: 'Synthetic example', translation: '合成例句' }] }] };
  const result = readApiEntry({ entry, suggestions: ['other'] }, 'terminal'); assert.equal(result.word, 'terminal'); assert.equal(result.definitions.length, 1); assert.equal(result.definitions[0].examples[0].translation, '合成例句');
});

test('all lookup inputs share punctuation trimming without salvaging paths or commands', () => {
  const cases=require('./selection-cases.json');
  for(const {raw,expected} of cases){
    if(expected===null)assert.throws(()=>normalizeSelection(raw),Error,JSON.stringify(raw));
    else {
      assert.equal(normalizeSelection(raw),expected,JSON.stringify(raw));
      assert.equal(normalizeSelection(expected),expected,`normalization must be stable: ${JSON.stringify(raw)}`);
    }
  }
});
