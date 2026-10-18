import http from 'node:http';
import { readFile } from 'node:fs/promises';
import { resolve, extname } from 'node:path';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
// Optional browser verification; no production bundler or dependency is added.
// SHINY_PLAYWRIGHT_MODULE=/path/to/playwright/package.json node scripts/test-responsive-ui.mjs
const { chromium } = createRequire(process.env.SHINY_PLAYWRIGHT_MODULE || import.meta.url)('playwright');
const root=fileURLToPath(new URL('../',import.meta.url));
const failures=[];
const imageFixture={image_id:'test',title:'Adjustment test',width:2,height:2,adjust:{brightness:0},looks:Array.from({length:16},(_,i)=>({key:`look-${i}`,name:`Look ${i+1}`}))};
const theme=process.env.SHINY_UI_THEME || 'pitch';
assert.ok(['pitch','light','noir','neumorphic','neumorphic-light'].includes(theme));
const server=http.createServer(async(req,res)=>{
  const u=new URL(req.url,'http://localhost');
  if(u.pathname==='/test') {res.setHeader('Content-Type','text/html');res.end(`<!doctype html><html data-theme="${theme}"><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="/themes/${theme}/tokens.css"><link rel="stylesheet" href="/ui/ui.css"><link rel="stylesheet" href="/themes/${theme}/components.css"><link rel="stylesheet" href="/css/app.css"><link rel="stylesheet" href="/css/tiles.css"><link rel="stylesheet" href="/css/settings.css"><link rel="stylesheet" href="/css/keyboard.css"><link rel="stylesheet" href="/css/plugins.css"><link rel="stylesheet" href="/themes/${theme}/app.css"><link rel="stylesheet" href="/ui/flow.css"><link rel="stylesheet" href="/ui/responsive.css"><style>body{margin:0}#fixture{position:fixed;inset:8px;display:flex}#fixture>.tile{flex:1;min-width:0;min-height:0}</style><body><main id="fixture"></main></body></html>`);return;}
  if(u.pathname.startsWith('/api/')) {
    if(/^\/api\/images\/test\/(render|apply)$/.test(u.pathname)) {
      res.setHeader('x-image-width','2'); res.setHeader('x-image-height','2');
      res.setHeader('x-image-adjust',JSON.stringify(imageFixture.adjust));
      res.end(Buffer.from([150,180,220,255,170,200,230,255,60,100,130,255,80,120,150,255]));return;
    }
    let data=[];
    if(u.pathname==='/api/images') data=[imageFixture];
    if(u.pathname==='/api/images/test') data=imageFixture;
    if(u.pathname==='/api/images/test/looks') {res.setHeader('Content-Type','image/svg+xml');res.end('<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="64"><rect width="1024" height="64" fill="#789abc"/></svg>');return;}
    if(u.pathname.includes('/files/list')) data={entries:[],path:''};
    if(u.pathname==='/api/mail/status') data={accounts:[{id:'test',email:'person@example.test',label:'Personal account',verified:true}],configured:true,presets:[]};
    if(u.pathname==='/api/mail/message') data={subject:'HTML email',from:['person@example.test'],to:['you@example.test'],html:'<html><body style="font-family: Georgia, serif; color: rgb(123, 45, 67)"><p>Original message typography</p></body></html>'};
    if(u.pathname==='/api/mail/folders') data={folders:[{name:'INBOX',count:3},{name:'Sent',count:0},{name:'Drafts',count:1},{name:'Archive',count:2}]};
    if(u.pathname==='/api/mail/list') data={messages:[{id:1,uid:1,subject:'A message with a long subject for mobile layouts',from:'person@example.test',date:'2026-10-17',seen:false}]};
    if(u.pathname==='/api/plugins') data=['word','calc','mail','browser'].map(name=>({name,description:'A useful plugin with a long description to exercise wrapping.',category:'Productivity',version:'1.0',api_level:1,surface:true}));
    if(u.pathname==='/api/plugins/active') data=['word','calc','mail','browser','keyboard'];
    if(u.pathname==='/api/documents') data=[{id:'test',title:'Example document'}];
    if(u.pathname==='/api/documents/test') data={id:'test',title:'Example document',html:'<h1>Example document</h1><p>A paragraph to edit on a phone.</p>'};
    if(u.pathname==='/api/spreadsheets') data=[{id:'test',title:'Example spreadsheet'}];
    if(u.pathname==='/api/spreadsheets/test') data={id:'test',title:'Example spreadsheet',cells:{A1:'Item',B1:'Amount',A2:'Example',B2:'120'}};
    if(u.pathname==='/api/presentations') data=[{id:'test',title:'Example presentation'}];
    if(u.pathname==='/api/presentations/test') data={id:'test',title:'Example presentation',theme:'aurora',slides:[{layout:'title',title:'A presentation on a phone',subtitle:'Portrait and landscape',notes:''},{layout:'content',title:'Another slide',bullets:['One','Two']}]};
    if(u.pathname==='/api/fonts') data=['system-ui','sans-serif','serif','monospace'];
    if(u.pathname.includes('/updates/status')) data={};
    res.setHeader('Content-Type','application/json');res.end(JSON.stringify({success:true,data}));return;
  }
  let path=u.pathname.startsWith('/plugins/') ? u.pathname.replace(/^\/plugins\/([^/]+)\//,'/plugins/$1/web/') : '/web'+u.pathname;
  // Image's editor imports from three levels up; its API imports from two.
  path=path.replace('/plugins/image/web/web/','/plugins/image/web/');
  try{let bytes=await readFile(resolve(root,'.'+path)); if(path==='/web/js/tiles.js') bytes=Buffer.from(bytes.toString().replace('function ensureWindowChrome(', 'export function ensureWindowChrome('));res.setHeader('Content-Type',({'.js':'text/javascript','.mjs':'text/javascript','.css':'text/css','.json':'application/json','.svg':'image/svg+xml'})[extname(path)]||'application/octet-stream');res.end(bytes);}catch{res.statusCode=404;res.end('missing '+path);}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const browser=await chromium.launch({headless:true,args:['--no-sandbox']});
try{
 const names=process.argv.slice(2);const plugins=names.length?names:['browser','calc','calculator','calendar','files','image','impress','mail','pdf','radio','studio','terminal','updates','word','youtube','settings','plugins','traveler','keyboard'];
 for(const size of [{width:360,height:740},{width:740,height:360},{width:320,height:568},{width:1280,height:800,touch:false},{width:1280,height:800,touch:false,tileWidth:360}]){
 const context=await browser.newContext({viewport:{width:size.width,height:size.height},isMobile:size.touch!==false,hasTouch:size.touch!==false});
 for(const name of plugins){
 const page=await context.newPage();const errors=[];const fonts=[];page.on('request',r=>{if(r.resourceType()==='font' && new URL(r.url()).origin!==new URL(page.url()).origin) fonts.push(r.url());});page.on('pageerror',e=>errors.push(e.message));
 await page.route('https://**',r=>r.abort());
 await page.goto(`http://127.0.0.1:${server.address().port}/test`);
 await page.evaluate(async()=>{const {setFont}=await import('/ui/index.js');setFont('DejaVu Serif');});
 try{
 if(size.tileWidth) await page.locator('#fixture').evaluate((e,w)=>{e.style.width=`${w}px`;},size.tileWidth);
 await page.evaluate(async name=>{
   if(name==='traveler') {
     const mod=await import('/js/tiles.js'); mod.mountMapTile();
     const tile=mod.getPluginTile('traveler'); document.querySelector('#fixture').append(tile);
     mod.ensureWindowChrome(tile,name);
     window.testModule={default:{unmount:mod.unmountMapTile}};return;
   }
   if(name==='keyboard') {
     const {initI18n}=await import('/js/i18n.js'); await initI18n();
     const {element,input}=await import('/ui/index.js');
     const tile=element('section','tile'); tile.append(input({placeholder:'Keyboard target',label:'Keyboard target'}));
     document.querySelector('#fixture').append(tile);
     const mod=await import('/js/keyboard.js'); mod.initKeyboard(); await mod.refreshKeyboard(); mod.toggleKeyboard();
     document.querySelector('#fixture').style.bottom='calc(var(--keyboard-height) + 12px)';
     window.testModule={default:{unmount:()=>mod.toggleKeyboard()}};return;
   }
   const path=['settings','plugins'].includes(name)?`/js/${name}Window.js`:`/plugins/${name}/plugin.js`;
   const {initI18n,loadPluginCatalog}=await import('/js/i18n.js'); await initI18n(); if(!['settings','plugins'].includes(name)) await loadPluginCatalog(new URL(path,location.href)); const mod=await import(path);window.testModule=mod;
   const surface=mod.default; const tile=surface.mount();
   document.querySelector('#fixture').append(tile);
   const {ensureWindowChrome}=await import('/js/tiles.js');
   ensureWindowChrome?.(tile,name);
 },name);
 await page.waitForTimeout(180);
 if(name==='files') await page.locator('button[aria-label="Search"]').first().click();
 if(name==='image') {
   await page.locator('.image-look-chip').first().waitFor();
   assert.equal(await page.locator('.ui-dial').count(),8,'all eight Image adjustments exist');
   const visible=await page.locator('.ui-dial').first().evaluate(e=>{
     const r=e.getBoundingClientRect(),dock=e.closest('.image-dock').getBoundingClientRect();
     const svg=e.querySelector('svg').getBoundingClientRect();
     return r.top>=dock.top && r.bottom<=dock.bottom && svg.width>=40 && svg.height>=40;
   });
   assert.ok(visible,'Image knobs are visible initially with a document and looks loaded');
 }
 const wrongFonts=await page.locator('.tile button,.tile input,.tile select,.tile textarea,.tile .ui-dial').evaluateAll(nodes=>nodes.filter(e=>e.getClientRects().length && !getComputedStyle(e).fontFamily.includes('DejaVu Serif')).map(e=>({className:e.className,font:getComputedStyle(e).fontFamily})).slice(0,25));
 assert.deepEqual(wrongFonts,[],'controls honor the font selected in Preferences');
 const searches=await page.locator('.tile .ui-search').evaluateAll(nodes=>nodes.map(e=>{
   const s=getComputedStyle(e.querySelector('input'));
   return Object.fromEntries(['height','fontFamily','fontSize','borderRadius','paddingTop','paddingBottom','paddingLeft','backgroundColor','borderTopWidth','borderTopColor','boxShadow'].map(k=>[k,s[k]]));
 }));
 const reference=await page.evaluate(async()=>{
   const {searchBar}=await import('/ui/index.js');const el=searchBar();document.body.append(el);
   const s=getComputedStyle(el.input),result=Object.fromEntries(['height','fontFamily','fontSize','borderRadius','paddingTop','paddingBottom','paddingLeft','backgroundColor','borderTopWidth','borderTopColor','boxShadow'].map(k=>[k,s[k]]));el.remove();return result;
 });
 for(const search of searches) assert.deepEqual(search,reference,'every search uses the shared rendered style');
 if(name==='mail') {
   await page.locator('.mail-item').first().click();
   const frame=page.locator('.mail-reader iframe'); await frame.waitFor();
   assert.ok((await frame.getAttribute('srcdoc')).includes('font-family: Georgia, serif; color: rgb(123, 45, 67)'), 'received email typography stays intact');
 }
 if(name==='keyboard') {
   const box=await page.locator('#keyboard-bar').boundingBox();
   assert.ok(box.x>=-1 && box.y>=0 && box.x+box.width<=size.width+1 && box.y+box.height<=size.height+1,'keyboard fits viewport');
 }

 const report=await page.evaluate(()=>{
   const root=document.querySelector('.tile');const rect=root.getBoundingClientRect();
   const over=[...root.querySelectorAll('*')].filter(e=>{
     const r=e.getBoundingClientRect();if(!r.width||!r.height||getComputedStyle(e).visibility==='hidden'||getComputedStyle(e).pointerEvents==='none')return false;
     if(r.right<=rect.right+2&&r.left>=rect.left-2)return false;
     for(let p=e.parentElement;p&&p!==root;p=p.parentElement)if(['auto','scroll','hidden'].includes(getComputedStyle(p).overflowX))return false;
     return true;
   }).slice(0,12).map(e=>e.className||e.tagName);
   const bars=[...root.querySelectorAll('[data-window-bar]')].map(e=>({height:e.clientHeight,scroll:e.scrollHeight,width:e.clientWidth,scrollWidth:e.scrollWidth}));
   const clipped = [...root.querySelectorAll('button,input:not([type="hidden"]),select,textarea,[role="button"],[role="slider"]')].filter(e => {
     if (e.getClientRects().length === 0 || getComputedStyle(e).visibility === 'hidden') return false;
     let box=e.getBoundingClientRect();
     if(!box.width || !box.height) return false;
     let x={left:box.left,right:box.right}, y={top:box.top,bottom:box.bottom};
     for(let p=e.parentElement;p && p!==document.body;p=p.parentElement) {
       const style=getComputedStyle(p), bounds=p.getBoundingClientRect();
       if(['auto','scroll'].includes(style.overflowX)) x={left:bounds.left,right:bounds.right};
       else if(['hidden','clip'].includes(style.overflowX) && (x.left<bounds.left-2 || x.right>bounds.right+2)) return true;
       if(['auto','scroll'].includes(style.overflowY)) y={top:bounds.top,bottom:bounds.bottom};
       else if(['hidden','clip'].includes(style.overflowY) && (y.top<bounds.top-2 || y.bottom>bounds.bottom+2)) return true;
     }
     return false;
   }).slice(0,15).map(e=>e.className || e.tagName);
   return {over,bars,clipped};
 });
 console.log(JSON.stringify({name,size,errors,...report}));
 if(errors.length || report.over.length || report.clipped.length || fonts.length) failures.push({name,size,errors,over:report.over,clipped:report.clipped,fonts});
 if(process.env.SHINY_UI_SCREENSHOTS) await page.screenshot({path:`${process.env.SHINY_UI_SCREENSHOTS}/${name}-${size.width}x${size.height}${size.tileWidth ? `-tile${size.tileWidth}` : ''}-${theme}.png`});
 if(name==='settings') {
  for(const id of ['account','appearance','desktop','browser','assistant','voice','images','music','power','touchbar','remote','system']) {
   await page.evaluate(id=>window.testModule.selectSettingsSection(id),id);
   const overflow=await page.evaluate(()=>{const p=document.querySelector('.settings-panels');return p.scrollWidth>p.clientWidth+2;});
   if(overflow) failures.push({name,size,section:id,overflow});
  }
 }
 // The shared select must work by touch, fit the viewport, and be dismissible.
 if(['word','calc','pdf','impress','mail'].includes(name)) {
  const trigger=page.locator('.ui-menu-trigger').first();
  if(size.touch===false) await trigger.click(); else await trigger.tap();
  const menu=page.locator('.ui-menu:not(.hidden)');
  await menu.waitFor({state:'visible'});
  const box=await menu.boundingBox();
  assert.ok(box.x>=0 && box.y>=0 && box.x+box.width<=size.width+1 && box.y+box.height<=size.height+1,'document/account menu fits the viewport');
  await page.keyboard.press('End');
  assert.ok(await menu.evaluate(e=>e.contains(document.activeElement)),'menu keeps keyboard focus on its actions');
  await page.keyboard.press('Escape');
  await menu.waitFor({state:'hidden'});
  assert.equal(await trigger.getAttribute('aria-expanded'),'false');
 }
 if(name==='plugins') {
  const select=page.locator('.ui-select').first();
  if(size.touch===false) await select.click(); else await select.tap();
  await page.locator('.ui-select-menu').waitFor({state:'visible'});
  const box=await page.locator('.ui-select-menu').boundingBox();
  assert.ok(box.x>=0 && box.y>=0 && box.x+box.width<=size.width+1 && box.y+box.height<=size.height+1,'dropdown fits the viewport');
  await page.keyboard.press('Escape');
  await page.evaluate(async()=>{ const ui=await import('/ui/index.js'); window.promptResult=ui.promptDialog('Rename', 'Before'); });
  await page.locator('.ui-modal.is-open input').fill('After');
  await page.locator('.ui-modal.is-open form').evaluate(form=>form.requestSubmit());
  assert.equal(await page.evaluate(()=>window.promptResult),'After');
  await page.evaluate(async()=>{ const ui=await import('/ui/index.js'); window.confirmResult=ui.confirmDialog('Remove this item?'); });
  await page.keyboard.press('Escape');
  assert.equal(await page.evaluate(()=>window.confirmResult),false,'Escape cancels without applying changes');
 }
 await page.evaluate(()=>window.testModule.default.unmount?.());
 }catch(e){console.log(JSON.stringify({name,size,failed:e.message,errors}));failures.push({name,size,failed:e.message});}
 await page.close();
 }
 await context.close();
 }
}finally{await browser.close();server.close();}

assert.deepEqual(failures, [], 'responsive browser regressions');
console.log('Responsive window checks passed');
