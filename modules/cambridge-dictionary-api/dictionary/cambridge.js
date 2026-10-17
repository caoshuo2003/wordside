const cheerio = require('cheerio');
const { createHttpClient } = require('../lib/http-client');
const { DictionaryError, invalidPage, notFound } = require('../lib/errors');
const SITE_URL = 'https://dictionary.cambridge.org';
const LANGUAGES = {
  en: { language: 'english', nation: 'us' },
  uk: { language: 'english', nation: 'uk' },
  'en-cn': { language: 'english-chinese-simplified', nation: 'us' },
  'en-tw': { language: 'english-chinese-traditional', nation: 'us' },
};
function buildDictionaryUrl(language, entry) {
  const option = Object.hasOwn(LANGUAGES, language) ? LANGUAGES[language] : undefined;
  if (!option) throw new DictionaryError(400, 'UNSUPPORTED_LANGUAGE', 'Unsupported language');
  if (typeof entry !== 'string' || !entry.trim() || entry.length > 120 || /[\x00-\x1f\x7f/\\]/.test(entry) || ['.', '..'].includes(entry.trim())) {
    throw new DictionaryError(400, 'INVALID_ENTRY', 'Invalid dictionary entry');
  }
  return `${SITE_URL}/${option.nation}/dictionary/${option.language}/${encodeURIComponent(entry.trim())}`;
}

function loadDocument(html) {
  if (typeof html !== 'string' || !/<\/html\s*>\s*(?:<!--[\s\S]*?-->\s*)*$/i.test(html)) {
    throw invalidPage('missing complete HTML document');
  }
  const $ = cheerio.load(html);
  if ($('#challenge-form, #cf-chl-widget, #cf-challenge-running').length || /just a moment|access denied|verify you are human/i.test($('title').text())) {
    throw new DictionaryError(502, 'CAMBRIDGE_BLOCKED', 'Cambridge requires verification or denied the request');
  }
  return $;
}
function buildSuggestionUrl(language, entry) {
  const url = new URL(buildDictionaryUrl(language, entry));
  url.pathname = `/${LANGUAGES[language].nation}/spellcheck/${LANGUAGES[language].language}/`;
  url.searchParams.set('q', entry.trim());
  return url.href;
}
const MAX_SUGGESTIONS = 10;
const normalizeWord = (value) => value.replace(/’/g, "'").replace(/\s+/g, ' ').trim();
function parseCambridgeSuggestions(html, language, entry) {
  const expected = new URL(buildSuggestionUrl(language, entry));
  const $ = loadDocument(html);
  const source = $('meta[property="og:url"]').attr('content');
  const allowedPath = (path) => path === expected.pathname || (language === 'uk' && path === expected.pathname.replace('/uk/', '/'));
  let sourceUrl;
  try { sourceUrl = new URL(source); } catch { throw invalidPage('missing spellcheck source'); }
  if (sourceUrl.origin !== SITE_URL || sourceUrl.username || sourceUrl.password || !allowedPath(sourceUrl.pathname) ||
      normalizeWord($('#searchword').attr('value') || '').toLowerCase() !== normalizeWord(entry).toLowerCase()) {
    throw invalidPage('mismatched spellcheck source or query');
  }
  // Scope to the site's explicit spelling list. Browse ranges and navigation
  // on the no-suggestions page must never become proposed dictionary entries.
  const heading = $('h1').filter((_, node) => /^Search suggestions for\s/.test($(node).text())).first();
  if (!heading.length) {
    if ($('h1').text().trim() === 'Your search terms did not match any entries.') return [];
    throw invalidPage('missing spellcheck heading');
  }
  if (normalizeWord(heading.find('span.tb').text()).toLowerCase() !== normalizeWord(entry).toLowerCase()) {
    throw invalidPage('mismatched spellcheck heading');
  }
  const list = heading.nextAll('ul.hul-u').first();
  if (!list.length) throw invalidPage('missing spellcheck list');
  const suggestions = [], seen = new Set([normalizeWord(entry).toLowerCase()]);
  list.children('li').children('a[href]').each((_, node) => {
    let url;
    try { url = new URL($(node).attr('href'), SITE_URL); } catch { return; }
    const path = expected.pathname.replace('/spellcheck/', '/search/') + 'direct/';
    if (url.origin !== SITE_URL || url.username || url.password ||
        !(url.pathname === path || (language === 'uk' && url.pathname === path.replace('/uk/', '/'))) ||
        url.searchParams.getAll('q').length !== 1) return;
    const word = normalizeWord($(node).text()), key = word.toLowerCase();
    if (!word || word.length > 80 || word.split(' ').length > 5 ||
        !/^[a-z0-9][a-z0-9 '-]*$/i.test(word) || !/[a-z]/i.test(word) ||
        normalizeWord(url.searchParams.get('q')).toLowerCase() !== key || seen.has(key)) return;
    if (suggestions.length < MAX_SUGGESTIONS) { seen.add(key); suggestions.push(word); }
  });
  return suggestions;
}

// Extracted from Nongfsq/cambridge-dictionary-api/data.js. Field names,
// selectors, ordering and text() behavior are retained.
function parseCambridgePage(html) {
  const $ = loadDocument(html);
  // Cambridge also serves complete standalone phrase entries (e.g. minutes).
  // Only use the phrase's own title; navigation/headings cannot supply a word.
  const word = $('.hw.dhw').first().text() || $('.phrase-di-block.dphrase-di-block > .di-title .headword.dhw').first().text();
  if (!word) {
    // Cambridge redirects an unknown word to a dictionary landing page (200).
    // A missing headword in an arbitrary or broken page is an upstream error.
    const canonical = $('link[rel="canonical"]').attr('href') || '';
    if (/^https:\/\/dictionary\.cambridge\.org\/(?:us\/|uk\/)?dictionary\/english(?:-chinese-simplified|-chinese-traditional)?\/$/.test(canonical)) throw notFound();
    throw invalidPage('missing headword');
  }
  const posElements = $('.pos.dpos');
  const pos = [...new Set(posElements.map((i, el) => $(el).text()).get())];
  const audio = [];
  $('.pos-header.dpos-h').each((i, s) => {
    const posNode = $(s).find('.dpos-g').first();
    if (!posNode.length) return;
    const p = posNode.text();
    $(s).find('.dpron-i').each((j, node) => {
      const $node = $(node);
      const lang = $node.find('.region.dreg').text();
      const audioSrc = $node.find('audio source').attr('src');
      const pron = $node.find('.pron.dpron').text();
      if (audioSrc && pron) audio.push({ pos: p, lang, url: SITE_URL + audioSrc, pron });
    });
  });
  const blocks = $('.def-block.ddef_block');
  if (!blocks.length) throw invalidPage('missing definitions');
  let assignedBodies = 0;
  const definition = blocks.map((index, element) => {
    const $element = $(element);
    const pos = $element.closest('.pr.entry-body__el, .phrase-di-block.dphrase-di-block').find('.pos.dpos').first().text();
    const source = $element.closest('.pr.dictionary').attr('data-id');
    const text = $element.find('.def.ddef_d.db').text();
    const translation = $element.find('.def-body.ddef_b > span.trans.dtrans').text();
    const bodyCount = $element.find('.def-body.ddef_b').length;
    // Cambridge's complete American English entries can contain a definition
    // without a body when there are no translations/examples (e.g. term/run).
    // Reject lost/moved ancillary content; do not discard the bare definition.
    if ($element.find('.def.ddef_d.db').length !== 1 || bodyCount > 1 || !text.trim() ||
        (bodyCount === 0 && $element.find('.examp.dexamp').length) ||
        $element.find('.trans.dtrans').length !== $element.find('.def-body.ddef_b .trans.dtrans').length) {
      throw invalidPage('incomplete definition structure');
    }
    assignedBodies += bodyCount;
    const examples = $element.find('.def-body.ddef_b > .examp.dexamp');
    if (examples.length !== $element.find('.examp.dexamp').length) throw invalidPage('unassigned examples');
    const example = examples.map((i, ex) => {
      const $ex = $(ex);
      if ($ex.find('.eg.deg').length !== 1 || !$ex.find('.eg.deg').text().trim()) throw invalidPage('incomplete example');
      return { id: i, text: $ex.find('.eg.deg').text(), translation: $ex.find('.trans.dtrans').text() };
    }).get();
    return { id: index, pos, source, text, translation, example };
  }).get();
  if ($('.def.ddef_d.db').length !== definition.length || $('.def-body.ddef_b').length !== assignedBodies) {
    throw invalidPage('unassigned definitions');
  }
  return { word, pos, pronunciation: audio, definition };
}
function requestError(error) {
  const status = error.response?.status;
  if (status === 404) return notFound();
  if (['ECONNABORTED', 'ETIMEDOUT', 'ERR_CANCELED'].includes(error.code)) return new DictionaryError(504, 'CAMBRIDGE_TIMEOUT', 'Cambridge request timed out');
  return new DictionaryError(502, status === 403 || status === 429 ? 'CAMBRIDGE_BLOCKED' : 'CAMBRIDGE_UNAVAILABLE', status ? `Cambridge returned HTTP ${status}` : 'Unable to connect to Cambridge');
}
function createCambridge({ client = createHttpClient() } = {}) {
  return {
    async lookup(language, entry) {
      const url = buildDictionaryUrl(language, entry), deadline = Date.now() + 10000;
      try {
        let response;
        try { response = await client.get(url, { signal: AbortSignal.timeout(10000) }); }
        catch (error) { throw requestError(error); }
        if (response.status === 404) throw notFound();
        if (response.status !== 200) throw new DictionaryError(502, 'CAMBRIDGE_UNAVAILABLE', `Cambridge returned HTTP ${response.status}`);
        return parseCambridgePage(response.data);
      } catch (error) {
        if (!(error instanceof DictionaryError) || error.status !== 404 || error.code !== 'WORD_NOT_FOUND') throw error;
        // Suggestions are optional, only after a confirmed missing word. Keep
        // the existing total request budget; normal hits perform a single GET.
        const remaining = Math.min(3000, deadline - Date.now());
        if (remaining > 0) {
          try {
            const response = await client.get(buildSuggestionUrl(language, entry), { signal: AbortSignal.timeout(remaining) });
            if (response.status === 200) {
              const suggestions = parseCambridgeSuggestions(response.data, language, entry);
              if (suggestions.length) error.suggestions = suggestions;
            }
          } catch { /* An optional suggestion failure cannot change the exact lookup outcome. */ }
        }
        throw error;
      }
    },
    close() { client.close?.(); },
  };
}
module.exports = { buildDictionaryUrl, buildSuggestionUrl, parseCambridgePage, parseCambridgeSuggestions, createCambridge, MAX_SUGGESTIONS };
