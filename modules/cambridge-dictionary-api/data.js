const express = require('express');
const cors = require('cors');
const path = require('node:path');
const { createLookup } = require('./lookup');
const { DictionaryError } = require('./lib/errors');

function createApp({ lookup = createLookup() } = {}) {
  const app = express();
  app.disable('x-powered-by');
  app.use(cors({ origin: '*' }));
  app.get('/', (req, res) => res.sendFile(path.join(__dirname, 'index.html')));
  // Process health only; this does not claim Cambridge is reachable.
  app.get('/healthz', (req, res) => res.json({ status: 'ok' }));
  app.get('/api/dictionary/:language/:entry', async (req, res, next) => {
    try { res.status(200).json(await lookup(req.params.language, req.params.entry)); }
    catch (error) { next(error); }
  });
  app.use((error, req, res, next) => {
    if (res.headersSent) return next(error);
    if (error instanceof DictionaryError) return res.status(error.status).json({ error: error.message, code: error.code, ...(error.status === 404 && error.code === 'WORD_NOT_FOUND' && error.suggestions?.length ? { suggestions: error.suggestions } : {}) });
    if (error instanceof URIError) return res.status(400).json({ error: 'Invalid dictionary entry', code: 'INVALID_ENTRY' });
    console.error('API Error:', error.name || 'Error');
    return res.status(500).json({ error: 'Internal server error' });
  });
  app.locals.close = () => lookup.close?.();
  return app;
}
// Keep the original `require('./data')` Express-app export (including Vercel).
const app = createApp();
module.exports = app;
module.exports.createApp = createApp;
