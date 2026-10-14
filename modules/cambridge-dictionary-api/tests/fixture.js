// Synthetic content, not a copy of a Cambridge entry.
const html = `<!doctype html><html><head><title>EXAMPLE - Cambridge Dictionary</title>
<link rel="canonical" href="https://dictionary.cambridge.org/dictionary/english-chinese-simplified/example"></head><body>
<div class="pr dictionary" data-id="SYNTHETIC"><div class="entry-body"><div class="pr entry-body__el">
<div class="pos-header dpos-h"><span class="headword"><span class="hw dhw">example</span></span><div class="dpos-g"><span class="pos dpos">noun</span></div>
<span class="uk dpron-i"><span class="region dreg">uk</span><span class="pron dpron">/test/</span><audio><source src="/media/test.mp3"></audio></span>
<span class="us dpron-i"><span class="region dreg">us</span><span class="pron dpron">/test/</span><audio><source src="/us/media/test.mp3"></audio></span></div>
<div class="def-block ddef_block"><div class="def ddef_d db">A synthetic <b>meaning</b> for a test.</div><div class="def-body ddef_b"><span class="trans dtrans">测试<span class="dtrans">释义</span></span><span class="trans dtrans">；补充译文</span>
<div class="examp dexamp"><span class="eg deg">This is <b>one</b> example.</span><span class="trans dtrans">这是一个例句。</span></div><div class="examp dexamp"><span class="eg deg">An untranslated example.</span></div></div></div>
<div class="def-block ddef_block"><div class="def ddef_d db">Another synthetic meaning.</div><div class="def-body ddef_b"><div class="examp dexamp"><span class="eg deg">A second sense example.</span><span class="trans dtrans">另一个释义的例句。</span></div></div></div>
</div></div></div><div class="daccord"><span class="eg dexamp">Additional unassigned corpus example.</span></div></body></html>`;
const landing = '<html><head><title>Cambridge dictionary</title><link rel="canonical" href="https://dictionary.cambridge.org/dictionary/english-chinese-simplified/"></head><body>Dictionary index</body></html>';
function suggestionPage({ query = 'termnal', language = 'english-chinese-simplified', nation = 'us', words = ['terminal', 'terminals'], empty = false } = {}) {
  const escape = text => text.replace(/[&<>"']/g, char => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
  const heading = empty ? '<h1>Your search terms did not match any entries.</h1>' : `<h1 class="lpb-10 lbb">Search suggestions for <span class="tb">${escape(query)}</span></h1>`;
  const links = empty ? '<li><a href="https://dictionary.cambridge.org/us/browse/english/t/">browse range</a></li>' : words.map(word => `<li><a href="https://dictionary.cambridge.org/${nation}/search/${language}/direct/?q=${encodeURIComponent(word)}">${escape(word)}</a></li>`).join('');
  return `<html><head><title>Synthetic Cambridge spellcheck</title><meta property="og:url" content="https://dictionary.cambridge.org/${nation}/spellcheck/${language}/"></head><body><input id="searchword" value="${escape(query)}"><main><div>${heading}<p>Synthetic similar spellings</p><ul class="hul-u">${links}</ul></div></main><nav><a href="https://dictionary.cambridge.org/us/search/${language}/direct/?q=navigation">navigation</a></nav></body></html>`;
}
module.exports = { html, landing, suggestionPage };
