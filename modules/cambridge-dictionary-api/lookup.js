const { createCambridge, buildDictionaryUrl } = require('./dictionary/cambridge');
const { MemoryCache } = require('./lib/cache');

function createLookup({ cambridge = createCambridge(), cache = new MemoryCache() } = {}) {
  const pending = new Map();
  async function lookup(language, word) {
    const key = buildDictionaryUrl(language, word);
    const cached = cache.get(key);
    if (cached) return cached;
    let request = pending.get(key);
    if (!request) {
      request = (async () => {
        const entry = await cambridge.lookup(language, word);
        const result = {
          word: entry.word,
          pos: entry.pos,
          verbs: [], // Retained only for the original API JSON contract.
          pronunciation: entry.pronunciation,
          definition: entry.definition,
        };
        cache.set(key, result);
        return result;
      })().finally(() => pending.delete(key));
      pending.set(key, request);
    }
    // Library callers cannot mutate another caller's result or cached content.
    return structuredClone(await request);
  }
  lookup.close = () => { cambridge.close?.(); cache.clear(); };
  return lookup;
}
module.exports = { createLookup };
