// Development-only compatibility fixtures. All pages are synthetic, not scraped entries.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),zlib=require('node:zlib');
const root=path.resolve(__dirname,'..'),moduleRoot=path.join(root,'modules/cambridge-dictionary-api');
const {html,landing,suggestionPage}=require(path.join(moduleRoot,'tests/fixture'));
const {parseCambridgePage,parseCambridgeSuggestions}=require(path.join(moduleRoot,'dictionary/cambridge'));
const {upstreamResult}=require(path.join(moduleRoot,'tests/upstream-oracle'));
(async()=>{
 const pages=[],suggestions=[];
 async function page(name,html){
  const record={name,html};
  try {const entry=JSON.parse(JSON.stringify({...parseCambridgePage(html),verbs:[]}));assert.deepEqual(entry,(await upstreamResult(html,'en-cn')).body);record.entry=entry;}
  catch(error){if(!error.status)throw error;record.error={status:error.status,code:error.code};}
  pages.push(record);
 }
 await page('original-rich-entry',html);
 await page('preserve-whitespace-entities-and-duplicate-pos',html.replace('A synthetic <b>meaning</b>',' A  synthetic\n<b>meaning &amp; text</b> ').replace('</body>','<span class="pos dpos">noun</span></body>'));
 await page('source-is-optional',html.replace(' data-id="SYNTHETIC"',''));
 const bare='<div class="def-block ddef_block"><div class="ddef_h"><div class="def ddef_d db">A synthetic meaning without examples.</div></div></div>';
 const withBare=html.replace('</div></div></div><div class="daccord">',bare+'</div></div></div><div class="daccord">');
 await page('bare-english-definition',withBare);
 await page('complete-tail-comments',html+'\n<!-- synthetic --> \n');
 for(const [name,source]of [
  ['truncated',html.slice(0,-7)],['missing-def',html.replace('class="def ddef_d db"','class="broken"')],
  ['missing-body',html.replace('class="def-body ddef_b"','class="broken"')],['missing-example-text',html.replace('class="eg deg"','class="broken"')],
  ['orphan-body',html.replace('</body>','<div class="def-body ddef_b"></div></body>')],
  ['orphan-translation',html.replace('<div class="def-block ddef_block">','<div class="def-block ddef_block"><span class="trans dtrans">Orphan.</span>')],
  ['orphan-example',withBare.replace(bare,bare.replace('</div></div>','<div class="examp dexamp"><span class="eg deg">Moved.</span></div></div></div>'))],
  ['double-body',withBare.replace(bare,bare.replace('</div></div>','<div class="def-body ddef_b"></div><div class="def-body ddef_b"></div></div></div>'))],
  ['landing',landing],['arbitrary','<html><body>Unexpected response</body></html>'],['challenge-title',landing.replace('Cambridge dictionary','Just a moment')],
  ['challenge-form',html.replace('<body>','<body><form id="challenge-form"></form>')]
 ])await page(name,source);
 function suggestion(name,html,language='en-cn',query='termnal'){
  const record={name,html,language,query};try{record.words=parseCambridgeSuggestions(html,language,query);}catch(error){record.error={status:error.status,code:error.code};}suggestions.push(record);
 }
 for(const [language,dataset,nation]of [['en','english','us'],['uk','english','uk'],['en-cn','english-chinese-simplified','us'],['en-tw','english-chinese-traditional','us']]){
  suggestion('ordered-'+language,suggestionPage({language:dataset,nation}),language);
  suggestion('empty-'+language,suggestionPage({language:dataset,nation,empty:true}),language);
 }
 suggestion('heading-whitespace',suggestionPage().replace('Search suggestions for ', 'Search suggestions for\t'));
 suggestion('candidate-bom',suggestionPage({words:['\ufeffterminal\ufeff']}));
 suggestion('uk-redirect-root',suggestionPage({language:'english',nation:'uk'}).replaceAll('/uk/','/'),'uk');
 suggestion('limits',suggestionPage({words:['termnal','terminal','Terminal','terminals','bad/path','<img>','hello\nworld',...Array.from({length:20},(_,i)=>'candidate'+i)]}));
 const source=suggestionPage({words:['terminal']});
 for(const [name,page]of [
  ['foreign-link',source.replace('href="https://dictionary.cambridge.org/us/search','href="https://example.com/us/search')],
  ['wrong-dataset',source.replace('/search/english-chinese-simplified/','/search/english/')],
  ['wrong-label',source.replace('q=terminal','q=other')],['browse-link',source.replace('/search/','/browse/')],
  ['link-credentials',source.replace('href="https://dictionary.cambridge.org','href="https://user@dictionary.cambridge.org')],
  ['duplicate-q',source.replace('q=terminal','q=terminal&amp;q=other')],['truncated',source.slice(0,-7)],
  ['wrong-search',source.replace('value="termnal"','value="other"')],['foreign-source',source.replace('content="https://dictionary.cambridge.org','content="https://example.com')],
  ['source-credentials',source.replace('content="https://dictionary.cambridge.org','content="https://user@dictionary.cambridge.org')],
  ['wrong-heading',source.replace('<span class="tb">termnal','<span class="tb">other')],['missing-list',source.replace('class="hul-u"','class="changed"')],
  ['changed-heading',source.replace('Search suggestions for','Changed heading')],['challenge',source.replace('<body>','<body><form id="challenge-form"></form>')]
 ])suggestion(name,page);
 const output=path.join(root,'desktop/src-tauri/tests/fixtures');fs.mkdirSync(output,{recursive:true});
 fs.writeFileSync(path.join(output,'native-cambridge.json'),JSON.stringify({origin:'Synthetic compatibility fixtures generated with existing parser and original upstream oracle',pages,suggestions},null,2)+'\n');
 fs.writeFileSync(path.join(output,'entry.html.gz'),zlib.gzipSync(html));
 fs.writeFileSync(path.join(output,'oversize.html.gz'),zlib.gzipSync('x'.repeat(4*1024*1024+1)));
 console.log(JSON.stringify({pages:pages.length,suggestions:suggestions.length,originalOracle:pages.filter(x=>x.entry).length}));
})().catch(error=>{console.error(error);process.exitCode=1;});
