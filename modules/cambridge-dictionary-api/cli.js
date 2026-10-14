// Direct local module call: no Express listener, no public API intermediary.
const { createLookup } = require('./lookup');
const lookup = createLookup();
(async () => {
  const word = process.argv[2];
  const language = process.argv[3] || 'en-cn';
  if (!word) { lookup.close(); console.error('Usage: node cli.js "word or phrase" [en-cn|en|uk|en-tw]'); process.exitCode = 1; return; }
  try { console.log(JSON.stringify(await lookup(language, word), null, 2)); }
  catch (error) { console.error(JSON.stringify({ error: error.message, code: error.code || 'LOOKUP_FAILED', ...(error.status === 404 && error.code === 'WORD_NOT_FOUND' && error.suggestions?.length ? { suggestions: error.suggestions } : {}) })); process.exitCode = 1; }
  finally { lookup.close(); }
})().catch(() => { lookup.close(); process.exitCode = 1; });
