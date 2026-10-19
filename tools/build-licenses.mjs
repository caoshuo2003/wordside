// Build-time only. Collect source license notices from the locked Windows dependency graph.
import { readFileSync, writeFileSync, mkdirSync, readdirSync, existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { basename, dirname, join } from 'node:path';
const root=dirname(dirname(fileURLToPath(import.meta.url)));
const native=join(root,'desktop/src-tauri');
const metadata=spawnSync('cargo',['metadata','--locked','--format-version','1','--filter-platform','x86_64-pc-windows-msvc'],{cwd:native,encoding:'utf8',windowsHide:true,maxBuffer:32*1024*1024});
if(metadata.status!==0){process.stderr.write(metadata.stderr||'Cargo metadata failed\n');process.exit(metadata.status||1);}
const data=JSON.parse(metadata.stdout),packages=new Map(data.packages.map(pkg=>[pkg.id,pkg]));
const nodes=new Map(data.resolve.nodes.map(node=>[node.id,node]));
const selected=new Set();
function visit(id){if(selected.has(id))return;selected.add(id);for(const dep of nodes.get(id)?.deps||[])if(dep.dep_kinds.some(kind=>kind.kind!=='dev'))visit(dep.pkg);}
visit(data.resolve.root);
function licenseFiles(folder,depth=0){
 let files=[];
 for(const entry of readdirSync(folder,{withFileTypes:true})){
  const path=join(folder,entry.name);
  if(entry.isFile()&&/^(license|licence|copying|copyright)([-_.].*)?$/i.test(entry.name))files.push(path);
  else if(entry.isDirectory()&&depth<1&&/^(license|licences?|legal)$/i.test(entry.name))files.push(...licenseFiles(path,depth+1));
 }
 return files;
}
const version=JSON.parse(readFileSync(join(root,'desktop/package.json'),'utf8')).version;
const sections=[`WordSide ${version}\nWindows x64 native distribution; no Node runtime or JavaScript API dependencies are bundled.\nThis list includes native transitive/build dependencies conservatively.`,
 'Cambridge parser port derived from Nongfsq/cambridge-dictionary-api\n'+readFileSync(join(root,'modules/cambridge-dictionary-api/LICENSE.txt'),'utf8')];
const missing=[];
const fallback={
 'alloc-stdlib@0.3.0':['alloc-stdlib-BSD-3-Clause.txt'],
 'defmt-parser@1.0.0':['defmt-parser-MIT.txt','defmt-parser-APACHE.txt'],
 'selectors@0.38.0':['selectors-MPL-2.0.txt'],
 'webview2-com@0.39.1':['webview2-rs-MIT.txt'],
 'webview2-com-macros@0.8.1':['webview2-rs-MIT.txt'],
 'webview2-com-sys@0.39.1':['webview2-rs-MIT.txt'],
};
const notices=[];
for(const pkg of [...selected].map(id=>packages.get(id)).filter(pkg=>pkg.source).sort((a,b)=>a.name.localeCompare(b.name)||a.version.localeCompare(b.version))){
 const folder=dirname(pkg.manifest_path),files=licenseFiles(folder);
 if(pkg.license_file){const path=join(folder,pkg.license_file);if(existsSync(path)&&!files.includes(path))files.push(path);}
 if(!files.length)for(const name of fallback[pkg.name+'@'+pkg.version]||[]){const path=join(root,'third-party/licenses',name);if(existsSync(path))files.push(path);}
 if(!files.length){missing.push(`${pkg.name}@${pkg.version}`);continue;}
 notices.push({pkg,files:files.map(file=>({name:basename(file),text:readFileSync(file,'utf8').replace(/\r\n?/g,'\n').replace(/[ \t]+$/gm,'').trim()}))});
}
if(missing.length)throw new Error('Missing source license files: '+missing.join(', '));
// A notice whose text (copyright lines included) is identical across packages is printed once in the
// shared part below and referenced by number; every package keeps its own header and file names.
const uses=new Map();
for(const {pkg,files} of notices)for(const file of files)uses.set(file.text,[...(uses.get(file.text)||[]),`${pkg.name}@${pkg.version}`]);
const shared=new Map([...uses].filter(([,users])=>users.length>1).map(([text],i)=>[text,i+1]));
for(const {pkg,files} of notices){
 sections.push(`${pkg.name}@${pkg.version}\nLicense: ${pkg.license||'(see notice)'}\nSource: https://crates.io/crates/${pkg.name}/${pkg.version} (unmodified source available for this exact version)\nRepository: ${pkg.repository||pkg.homepage||''}\n`+
  files.map(file=>shared.has(file.text)?`${file.name}: identical to shared notice ${shared.get(file.text)} below.`:file.text).join('\n\n'));
}
for(const [text,number] of shared)sections.push(`Shared notice ${number}\nUsed by: ${[...new Set(uses.get(text))].join(', ')}\n\n${text}`);
const output=join(native,'generated');mkdirSync(output,{recursive:true});
writeFileSync(join(output,'THIRD-PARTY-LICENSES.txt'),sections.join('\n\n========================================\n\n'));
console.log(JSON.stringify({nativePackages:selected.size-1,licenseSections:sections.length,sharedNotices:shared.size,version,nodeRuntime:false}));
