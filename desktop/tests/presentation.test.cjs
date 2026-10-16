const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
const ts=require('typescript');
const shared=fs.readFileSync(require.resolve('../../shared/dictionary.ts'),'utf8');
const presentation=fs.readFileSync(require.resolve('../src/presentation.ts'),'utf8');
const context={URL,Error,DOMException};
vm.runInNewContext(ts.transpileModule(shared+'\n'+presentation+'\nglobalThis.product={readApiEntry,normalizeSelection,lookupNotice,captureNotice,shortcutNotice};',{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.None}}).outputText,context);
const {readApiEntry,normalizeSelection,lookupNotice,captureNotice,shortcutNotice}=context.product;
const raw='API HTTP 502 definition.translation UNKNOWN_OS_ERROR';
function failure(payload){try{readApiEntry(payload,'termnal');assert.fail('must reject payload');}catch(error){return error;}}
function notice(payload){return lookupNotice(failure(payload));}

test('missing words keep correction separate from connectivity or invalid content',()=>{
 const missing=notice({entry:null,apiError:raw,apiStatus:404,apiCode:'WORD_NOT_FOUND',suggestions:['terminal']});
 assert.equal(missing.title,'未找到这个词');assert.equal(missing.action,'correct');
 for(const [code,status] of [['CAMBRIDGE_UNAVAILABLE',502],['CAMBRIDGE_BLOCKED',502],['CAMBRIDGE_TIMEOUT',504],['INVALID_CAMBRIDGE_PAGE',502],['OTHER',502]]){
  const value=notice({entry:null,apiError:raw,apiStatus:status,apiCode:code,suggestions:['terminal']});
  assert.equal(value.action,'retry');assert.notEqual(value.title,missing.title);assert.ok(!JSON.stringify(value).includes(raw));assert.doesNotMatch(JSON.stringify(value),/API|HTTP|definition|502|504/);
 }
});
test('malformed successful JSON and candidate data never leak field diagnostics',()=>{
 for(const payload of [{entry:{}},{entry:{word:'terminal',pos:[],definition:'bad',pronunciation:[]}},{entry:null,apiError:raw,apiStatus:404,apiCode:'WORD_NOT_FOUND',suggestions:['bad/path']}]){
  const value=notice(payload);assert.equal(value.title,'释义暂时不可用');assert.equal(value.action,'retry');assert.doesNotMatch(JSON.stringify(value),/API|HTTP|字段|格式|数组/);
 }
});
test('unknown rejected strings and objects become ordinary product copy',()=>{
 for(const error of [raw,new Error(raw),{message:raw},null,undefined]){
  const value=lookupNotice(error);assert.equal(value.title,'查询暂未完成');assert.equal(value.action,'retry');assert.ok(!JSON.stringify(value).includes(raw));
 }
 assert.equal(lookupNotice(new DOMException(raw,'AbortError')).title,'查询有些慢');
});
test('invalid selected inputs guide correction without pretending the dictionary is offline',()=>{
 for(const value of ['two\nlines','/tmp/log.txt','a'.repeat(101)]){
  let error;try{normalizeSelection(value);}catch(reason){error=reason;}
  assert.ok(error);const message=lookupNotice(error);assert.equal(message.action,'correct');assert.equal(message.title,'请换一个查询词');
 }
 assert.equal(normalizeSelection('**terminal**'),'terminal');
});
test('capture states use concise local copy including unknown or inherited property names',()=>{
 for(const code of ['no_selection','unreported_selection','unsupported','protected','source_changed','invalid_selection','access_denied','read_error','copy_unavailable','clipboard_unavailable','clipboard_restore_failed','timeout','unknown','__proto__','constructor']){
  const value=captureNotice(code);assert.equal(typeof value.title,'string');assert.equal(typeof value.message,'string');assert.ok(value.message.length<40);assert.doesNotMatch(JSON.stringify(value),/API|接口|进程|权限|GetSelection/);
 }
 assert.notEqual(captureNotice('protected').message,captureNotice('no_selection').message);
});
test('shortcut failures preserve useful conflict guidance without exposing raw errors',()=>{
 assert.match(shortcutNotice('这组快捷键已被其他程序占用。原快捷键仍保留。'),/占用/);
 assert.match(shortcutNotice('快捷键服务未响应，请重新启动 WordSide。'),/重启/);
 assert.match(shortcutNotice('快捷键修饰键重复。'),/Ctrl 或 Alt/);
 for(const value of [raw,new Error(raw),undefined])assert.equal(shortcutNotice(value),'快捷键未能保存，请稍后再试。');
});
