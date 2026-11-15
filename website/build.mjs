import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { centralPublicKey, verifyBytes } from '../scripts/edgeone/prepare.mjs';
const root=path.dirname(fileURLToPath(import.meta.url)), output=path.join(root,'dist');
let update;
for(const base of ['https://null-object-0000.github.io/umanager/','https://updates.snewbie.site/umanager/']){
  try {
    const [data,sig]=await Promise.all([fetch(base+'v3/feed.json',{signal:AbortSignal.timeout(20000)}),fetch(base+'v3/feed.json.sig',{signal:AbortSignal.timeout(20000)})]);
    if(!data.ok||!sig.ok)throw Error('Feed unavailable');
    const bytes=Buffer.from(await data.arrayBuffer());
    if(bytes.length>1024*1024)throw Error('Feed oversized');
    verifyBytes(bytes,(await sig.text()).trim(),centralPublicKey());
    const entry=JSON.parse(bytes).selfUpdate;
    if(!entry||!/^\d+\.\d+\.\d+$/.test(entry.version)||entry.packageName!=='u-manager'||entry.architecture!=='amd64'||!/^[a-f0-9]{64}$/i.test(entry.sha256)||!Number.isSafeInteger(entry.size)||entry.size<=0)throw Error('Invalid signed release');
    const asset=`UManager_${entry.version}_amd64.deb`;
    if(entry.downloadUrl!==`https://github.com/null-object-0000/umanager/releases/download/v${entry.version}/${asset}`)throw Error('Unexpected release origin');
    update=entry;break;
  }catch(error){console.warn('Cannot use verified release from '+base+': '+error.message);}
}
if(!update)throw Error('No verified release is available; refusing to publish invented download metadata');
const origin='https://umanager.devtrends.site', repo='https://github.com/null-object-0000/umanager';
const mirror=`https://updates.snewbie.site/umanager/releases/v${update.version}/UManager_${update.version}_amd64.deb`;
const data={
 '@context':'https://schema.org','@graph':[
  {'@type':'Organization','@id':'https://devtrends.site/#organization',name:'DevTrends',url:'https://devtrends.site/'},
  {'@type':'WebSite','@id':origin+'/#website',name:'UManager',url:origin+'/',inLanguage:'zh-CN',publisher:{'@id':'https://devtrends.site/#organization'},isPartOf:{'@id':'https://devtrends.site/#website'}},
  {'@type':'SoftwareApplication','@id':origin+'/#software',name:'UManager',url:origin+'/',description:'面向 Ubuntu 的软件与开发环境管家，管理桌面应用、Node.js、Rust 与 AI 编程 CLI。',applicationCategory:'UtilitiesApplication',operatingSystem:'Ubuntu 22.04 or later (amd64)',softwareVersion:update.version,downloadUrl:mirror,releaseNotes:repo+'/releases/tag/v'+update.version,image:origin+'/assets/app-preview.jpg',publisher:{'@id':'https://devtrends.site/#organization'},offers:{'@type':'Offer',price:'0',priceCurrency:'CNY'}}
 ]
};
fs.mkdirSync(output,{recursive:true});
fs.cpSync(path.join(root,'assets'),path.join(output,'assets'),{recursive:true});
fs.copyFileSync(path.join(root,'../src/assets/umanager-logo.png'),path.join(output,'assets/logo.png'));
for(const asset of ['vscode.png','google-chrome.png','wechat.png','nodejs.svg','rust.svg','codex.svg'])fs.copyFileSync(path.join(root,'../src/assets/app-icons',asset),path.join(output,'assets',asset));
const replacements={VERSION:update.version,MIRROR_URL:mirror,GITHUB_URL:update.downloadUrl,RELEASE_URL:repo+'/releases/tag/v'+update.version,SHA256:update.sha256,SIZE_MB:(update.size/1e6).toFixed(1),STRUCTURED_DATA:JSON.stringify(data).replace(/</g,'\\u003c')};
let html=fs.readFileSync(path.join(root,'index.html'),'utf8').replace(/\{\{([A-Z0-9_]+)\}\}/g,(_,name)=>{if(!(name in replacements))throw Error('Unknown template field '+name);return replacements[name];});
if(/\{\{/.test(html))throw Error('Unrendered website template');
fs.writeFileSync(path.join(output,'index.html'),html);
for(const name of ['style.css','site.js'])fs.copyFileSync(path.join(root,name),path.join(output,name));
fs.writeFileSync(path.join(output,'robots.txt'),`User-agent: *\nAllow: /\nSitemap: ${origin}/sitemap.xml\n`);
fs.writeFileSync(path.join(output,'sitemap.xml'),`<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><url><loc>${origin}/</loc></url></urlset>`);
fs.writeFileSync(path.join(output,'release.json'),JSON.stringify({version:update.version,mirrorUrl:mirror,githubUrl:update.downloadUrl,sha256:update.sha256,size:update.size},null,2));
fs.writeFileSync(path.join(output,'404.html'),'<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>页面未找到 · UManager</title><meta name="robots" content="noindex"><h1>页面未找到</h1><a href="/">返回 UManager 官网</a></html>');
fs.writeFileSync(path.join(output,'edgeone.json'),JSON.stringify({headers:[{source:'/*',headers:[{key:'X-Content-Type-Options',value:'nosniff'},{key:'Referrer-Policy',value:'strict-origin-when-cross-origin'}]},{source:'/release.json',headers:[{key:'Cache-Control',value:'public,max-age=0,must-revalidate'}]}]}));
console.log('Built UManager website from verified release v'+update.version);
