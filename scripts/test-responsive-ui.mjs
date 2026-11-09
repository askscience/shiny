import http from 'node:http';
import { existsSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { resolve, extname } from 'node:path';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
// Optional browser verification; no production bundler or dependency is added.
// SHINY_PLAYWRIGHT_MODULE=/path/to/playwright/package.json node scripts/test-responsive-ui.mjs
// SHINY_UI_LOCALE=it (any catalog code) runs the windows in that language.
const { chromium } = createRequire(process.env.SHINY_PLAYWRIGHT_MODULE || import.meta.url)('playwright');
const root=fileURLToPath(new URL('../',import.meta.url));
const failures=[];
const imageFixture={image_id:'test',title:'Adjustment test',width:2,height:2,adjust:{brightness:0},looks:Array.from({length:16},(_,i)=>({key:`look-${i}`,name:`Look ${i+1}`}))};
const theme=process.env.SHINY_UI_THEME || 'dusk';
// A one-page PDF with a line of text, assembled with a correct xref so pdf.js loads it without repair.
const pdfFixture=(()=>{
  const content='BT /F1 24 Tf 72 740 Td (A page on a phone) Tj ET';
  const objs=['<< /Type /Catalog /Pages 2 0 R >>','<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',`<< /Length ${content.length} >>\nstream\n${content}\nendstream`];
  let out='%PDF-1.4\n';const offsets=[];
  objs.forEach((o,i)=>{offsets.push(out.length);out+=`${i+1} 0 obj\n${o}\nendobj\n`;});
  const xref=out.length;
  out+=`xref\n0 ${objs.length+1}\n0000000000 65535 f \n${offsets.map(o=>`${String(o).padStart(10,'0')} 00000 n \n`).join('')}trailer\n<< /Size ${objs.length+1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(out,'latin1');
})();
// Any theme shipped in web/themes (dusk, ink, editorial, …) can be measured.
assert.ok(['pitch','light','noir','neumorphic','neumorphic-light'].includes(theme) || existsSync(`${root}web/themes/${theme}/tokens.css`), `unknown theme ${theme}`);
const server=http.createServer(async(req,res)=>{
  const u=new URL(req.url,'http://localhost');
  if(u.pathname==='/test') {res.setHeader('Content-Type','text/html');res.end(`<!doctype html><html data-theme="${theme}"><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="/themes/${theme}/tokens.css"><link rel="stylesheet" href="/ui/ui.css"><link rel="stylesheet" href="/themes/${theme}/components.css"><link rel="stylesheet" href="/css/app.css"><link rel="stylesheet" href="/css/tiles.css"><link rel="stylesheet" href="/css/settings.css"><link rel="stylesheet" href="/css/settings-previews.css"><link rel="stylesheet" href="/css/keyboard.css"><link rel="stylesheet" href="/css/plugins.css"><link rel="stylesheet" href="/themes/${theme}/app.css"><link rel="stylesheet" href="/ui/flow.css"><link rel="stylesheet" href="/ui/responsive.css"><link rel="stylesheet" href="/css/mobile.css"><style>body{margin:0}#fixture{position:fixed;inset:8px;display:flex}#fixture>.tile{flex:1;min-width:0;min-height:0}</style><body><main id="fixture"></main></body></html>`);return;}
  if(u.pathname.startsWith('/api/')) {
    if(/^\/api\/images\/test\/(render|apply)$/.test(u.pathname)) {
      res.setHeader('x-image-width','2'); res.setHeader('x-image-height','2');
      res.setHeader('x-image-adjust',JSON.stringify(imageFixture.adjust));
      res.end(Buffer.from([150,180,220,255,170,200,230,255,60,100,130,255,80,120,150,255]));return;
    }
    if(u.pathname==='/api/pdfs/test/file') {res.setHeader('Content-Type','application/pdf');res.end(pdfFixture);return;}
    let data=[];
    if(u.pathname==='/api/images') data={images:[imageFixture],count:1};
    if(u.pathname==='/api/pdfs') data=[{pdf_id:'test',title:'Example PDF',page_count:1,updated_at:'2026-11-06T09:00:00Z'}];
    if(u.pathname==='/api/pdfs/test') data={pdf_id:'test',title:'Example PDF',page_count:1,updated_at:'2026-11-06T09:00:00Z',history:{undo:0,redo:0}};
    if(u.pathname==='/api/pdfs/test/comments') data={comments:[{page:0,index:0,kind:'Highlight',text:'A comment long enough to wrap on a phone screen',rect:[72,730,300,750],color:[1,0.85,0.2]}],marks:['watermark']};
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
    // Updates: two pending packages and a job that has just finished, so the
    // install action, the progress block and the Details pane are measured.
    if(u.pathname.includes('/updates/status')) data={count:2,distro:{id:'debian',manager:'apt',supported:true},ollama:{installed:true,version:'0.5.0',path:'/usr/local/bin/ollama'},packages:[{name:'liteaura',current:'1.2.0',candidate:'1.3.0',repo:'main'},{name:'linux-image-amd64',current:'6.12.1-1',candidate:'6.12.2-1',repo:'main'}]};
    if(u.pathname==='/api/updates/jobs') data=u.searchParams.has('from')?{job:'test',done:true,success:true,total:2,lines:['Reading package lists…','Unpacking liteaura (1.3.0)…'],progress:{phase:'install',percent:100,packages:[{name:'liteaura',state:'done'},{name:'linux-image-amd64',state:'done'}],failed:[]}}:{job:'test',done:false};
    if(u.pathname==='/api/updates/history') data={history:[{at:'2026-11-06 09:12',action:'apply',manager:'apt',status:'ok',detail:'2 packages'}]};
    if(u.pathname==='/api/installer/health') data={version:'test',protocol:4,dry_run:true,root:false,socket:'',hardware:{vendor:'Test',product:'Synthetic machine',board:'',is_t2:false,firmware:'uefi',secure_boot:false,gpus:[],live_medium:null}};
    if(u.pathname==='/api/installer/options') data={locales:['en_US.UTF-8','it_IT.UTF-8'],keyboards:['us','it'],timezones:['UTC','Europe/Rome'],default_locale:'it_IT.UTF-8',default_keyboard:'it',default_timezone:'Europe/Rome'};
    if(u.pathname==='/api/installer/probe') data={
      disks:[{
        name:'nvme0n1', path:'/dev/nvme0n1', kind:'disk',
        size_bytes:1000*1024**3, start_bytes:0, model:'Synthetic SSD', serial:'TEST-SERIAL',
        tran:'nvme', removable:false, read_only:false, is_loop:false, is_rom:false,
        mountpoints:[], fstype:'', pttype:'gpt', partuuid:'', uuid:'', label:'', fsver:'',
        parttypename:'', partflags:[], holders:[],
        free_extents:[{ start_bytes:300*1024**3+1024**2, size_bytes:700*1024**3-1024**2 }],
        children:[
          { name:'nvme0n1p1', path:'/dev/nvme0n1p1', kind:'part', size_bytes:200*1024**3,
            start_bytes:1024**2, model:'', serial:'', tran:'', removable:false, read_only:false,
            is_loop:false, is_rom:false, mountpoints:[], fstype:'ntfs', pttype:'gpt',
            partuuid:'p1', uuid:'u1', label:'Windows', fsver:'', parttypename:'Microsoft basic data',
            partflags:[], holders:[], free_extents:[], children:[] },
          { name:'nvme0n1p2', path:'/dev/nvme0n1p2', kind:'part', size_bytes:100*1024**3,
            start_bytes:200*1024**3+1024**2, model:'', serial:'', tran:'', removable:false, read_only:false,
            is_loop:false, is_rom:false, mountpoints:[], fstype:'ext4', pttype:'gpt',
            partuuid:'p2', uuid:'u2', label:'Data', fsver:'', parttypename:'Linux filesystem',
            partflags:[], holders:[], free_extents:[], children:[] }
        ]
      }],
      hardware:{vendor:'Test', product:'Synthetic machine', board:'', is_t2:false, firmware:'uefi',
        secure_boot:false, gpus:[], live_medium:null},
      live_medium:null
    };
    res.setHeader('Content-Type','application/json');res.end(JSON.stringify({success:true,data}));return;
  }
  let path=u.pathname.startsWith('/plugins/') ? u.pathname.replace(/^\/plugins\/([^/]+)\//,'/plugins/$1/web/') : '/web'+u.pathname;
  // Image's editor imports from three levels up; its API imports from two.
  path=path.replace('/plugins/image/web/web/','/plugins/image/web/');
  try{let bytes=await readFile(resolve(root,'.'+path)); if(path==='/web/js/tiles.js') bytes=Buffer.from(bytes.toString().replace('function ensureWindowChrome(', 'export function ensureWindowChrome(')
   // The fixture never "opens" the map window, so the tile lookup hands the mounted map tile back regardless.
   .replace('export function getPluginTile(name) {', 'export function getPluginTile(name) {\n  if (name === MAP_TILE_PLUGIN && mapTileEl) return mapTileEl;'));res.setHeader('Content-Type',({'.js':'text/javascript','.mjs':'text/javascript','.css':'text/css','.json':'application/json','.svg':'image/svg+xml'})[extname(path)]||'application/octet-stream');res.end(bytes);}catch{res.statusCode=404;res.end('missing '+path);}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const browser=await chromium.launch({headless:true,args:['--no-sandbox']});
try{
 const names=process.argv.slice(2);const plugins=names.length?names:['browser','calc','calculator','calendar','capture','files','image','impress','installer','mail','pdf','radio','studio','terminal','updates','word','youtube','settings','plugins','traveler','keyboard'];
 for(const size of [{width:360,height:740},{width:740,height:360},{width:320,height:568},{width:1280,height:800,touch:false},{width:1280,height:800,touch:false,tileWidth:360}]){
 const context=await browser.newContext({viewport:{width:size.width,height:size.height},isMobile:size.touch!==false,hasTouch:size.touch!==false});
 for(const name of plugins){
 const page=await context.newPage();const errors=[];const fonts=[];page.on('request',r=>{if(r.resourceType()==='font' && new URL(r.url()).origin!==new URL(page.url()).origin) fonts.push(r.url());});page.on('pageerror',e=>errors.push(e.message));
 await page.route('https://**',r=>r.abort());
 // SHINY_UI_LOCALE=it runs every window in that interface language.
 if(process.env.SHINY_UI_LOCALE) await page.addInitScript(code=>{try{localStorage.setItem('ui.locale',code);}catch(_){}},process.env.SHINY_UI_LOCALE);
 await page.goto(`http://127.0.0.1:${server.address().port}/test`);
 // A phone viewport is the phone layout (phoneShell.js / preferences.js mark
 // the document), so the phone rules — no title bar, icon-only actions, the
 // pane stack — are what gets measured.
 const phone=size.touch!==false;
 if(phone) await page.evaluate(()=>{document.documentElement.dataset.phone='1';document.documentElement.dataset.mobile='1';});
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
 if(name==='files') await page.locator('.files-search .ui-search-action').first().click();
 if(name==='installer') {
   // The wizard opens on the region step; the timeline is on the disk step.
   await page.locator('.installer-step-item').nth(1).click();
   await page.locator('.installer-timeline').waitFor();
   assert.ok(await page.locator('.installer-clip--planned').count()>=2,'the installer timeline renders planned clips');
   assert.ok(await page.locator('.installer-handle').count()>=1,'the installer timeline renders resize handles');
   // Narrow or phone: the summary is a pane over the step, opened from the
   // footer's info button and closed by Escape.
   if(await page.locator('.installer-shell').evaluate(e=>e.classList.contains('is-stacked'))) {
     const isOpen=()=>page.locator('.installer-manifest').evaluate(e=>e.classList.contains('is-open'));
     await page.locator('.installer-manifest-open').click();
     await page.waitForTimeout(120);
     assert.ok(await isOpen(),'the summary opens as a pane over the step');
     await page.keyboard.press('Escape');
     // Past the slide-out, so the closed pane is not measured in flight.
     await page.waitForTimeout(400);
     assert.ok(!await isOpen(),'Escape closes the summary pane');
   }
 }
 if(name==='image') {
   await page.locator('.image-look-chip').first().waitFor({state:'attached'});
   assert.equal(await page.locator('.ui-dial').count(),8,'all eight Image adjustments exist');
   const knobsVisible=(holder)=>page.locator('.ui-dial').first().evaluate((e,holder)=>{
     const r=e.getBoundingClientRect(),box=e.closest(holder).getBoundingClientRect();
     const svg=e.querySelector('svg').getBoundingClientRect();
     return r.top>=box.top && r.bottom<=box.bottom && svg.width>=40 && svg.height>=40;
   },holder);
   if(phone) {
     // Phone: the dock is a bottom sheet opened from the window bar's Adjust
     // icon; the knobs are measured inside it, then Escape closes it.
     assert.equal(await page.locator('.image-dock').evaluate(e=>e.hidden),true,'the inline dock is hidden on a phone');
     await page.locator('.image-tools-adjust').click();
     await page.waitForTimeout(250);
     assert.equal(await page.locator('.ui-sheet.is-open .ui-dial').count(),8,'the eight knobs are in the open sheet');
     assert.ok(await knobsVisible('.ui-sheet-card'),'Image knobs are visible in the sheet with a document and looks loaded');
     const sheetText=await page.locator('.ui-sheet.is-open .ui-btn').evaluateAll(nodes=>nodes.filter(e=>e.getClientRects().length&&[...e.childNodes].some(n=>n.nodeType===1&&!n.classList.contains('ui-icon')&&n.getClientRects().length&&n.textContent.trim())).map(e=>e.className));
     assert.deepEqual(sheetText,[],'the sheet shows icon-only one-clicks on a phone');
     await page.keyboard.press('Escape');
     await page.waitForTimeout(400);
     assert.equal(await page.locator('.ui-sheet.is-open').count(),0,'Escape closes the tools sheet');
   } else {
     assert.ok(await knobsVisible('.image-dock'),'Image knobs are visible initially with a document and looks loaded');
   }
 }
 // Tab names (`.ui-tab-label`, Terminal and Browser), the Browser's open-tab count, Calc's formula field, PDF's zoom readout and
 // Studio's mono capitals (its rail pages, browser tabs, zoom, M/S, tempo and step fields — the editorial skin) are mono by design.
  const wrongFonts=await page.locator('.tile button:not(.ui-tab-label,.pdf-zoom-label,.browser-tabcount,.image-zoom-chip,.terminal-key,.studio-head-btn,.studio-arr-addtrack,.studio-browser-tab,.studio-ms,.studio-seg>button),.tile input:not(.calc-formula-input,.calc-cell-editor,.studio-bpm,.studio-number),.tile select:not(.studio-select),.tile textarea:not(.xterm-helper-textarea),.tile .ui-dial').evaluateAll(nodes=>nodes.filter(e=>e.getClientRects().length && !getComputedStyle(e).fontFamily.includes('DejaVu Serif')).map(e=>({className:e.className,font:getComputedStyle(e).fontFamily})).slice(0,25));
 assert.deepEqual(wrongFonts,[],'controls honor the font selected in Preferences');
 const searches=await page.locator('.tile .ui-search:not(.radio-tile .ui-search)').evaluateAll(nodes=>nodes.map(e=>{
   const s=getComputedStyle(e.querySelector('input'));
   return Object.fromEntries(['height','fontFamily','fontSize','borderRadius','paddingTop','paddingBottom','paddingLeft','backgroundColor','borderTopWidth','borderTopColor','boxShadow'].map(k=>[k,s[k]]));
 }));
 const reference=await page.evaluate(async()=>{
   const {searchBar}=await import('/ui/index.js');const el=searchBar();document.body.append(el);
   const s=getComputedStyle(el.input),result=Object.fromEntries(['height','fontFamily','fontSize','borderRadius','paddingTop','paddingBottom','paddingLeft','backgroundColor','borderTopWidth','borderTopColor','boxShadow'].map(k=>[k,s[k]]));el.remove();return result;
 });
 for(const search of searches) assert.deepEqual(search,reference,'every search uses the shared rendered style');
 if(name==='mail') {
   await page.locator('.mail-row').first().click();
   const frame=page.locator('.mail-reader iframe'); await frame.waitFor();
   assert.ok((await frame.getAttribute('srcdoc')).includes('font-family: Georgia, serif; color: rgb(123, 45, 67)'), 'received email typography stays intact');
   // Narrow or phone: the open message is a pane over the list, Escape pops
   // it; it is reopened so the reader is what gets measured.
   const isOpen=()=>page.locator('.mail-reader').evaluate(e=>e.classList.contains('is-open'));
   if(await page.locator('.mail-shell').evaluate(e=>e.classList.contains('is-stacked'))) {
     assert.ok(await isOpen(),'the open message is the pane over the list');
     await page.keyboard.press('Escape');
     await page.waitForTimeout(120);
     assert.ok(!await isOpen(),'Escape closes the message pane');
     assert.equal(await page.locator('.mail-row.is-active').count(),0,'closing the pane deselects the message');
     await page.locator('.mail-row').first().click();
     await page.locator('.mail-reader iframe').waitFor();
     assert.ok(await isOpen(),'the message opens again');
     await page.waitForTimeout(400); // the slide-in
   }
 }
 if(name==='calculator') {
   // Narrow or phone: the history is a pane over the pad, opened from the
   // bar's History toggle and closed by Escape.
   const isOpen=()=>page.locator('.calculator-history').evaluate(e=>e.classList.contains('is-open'));
   if(await page.locator('.calculator-body').evaluate(e=>e.classList.contains('is-stacked'))) {
     await page.locator('.calculator-history-toggle').click();
     await page.waitForTimeout(400);
     assert.ok(await isOpen(),'the history opens as a pane over the keypad');
     assert.ok(await page.locator('.calculator-history .ui-pane-back').isVisible(),'the history pane has a Back');
     await page.keyboard.press('Escape');
     await page.waitForTimeout(400);
     assert.ok(!await isOpen(),'Escape closes the history pane');
     assert.equal(await page.locator('.calculator-history-toggle').getAttribute('aria-pressed'),'false','closing the pane releases the toggle');
   } else {
     await page.locator('.calculator-history-toggle').click();
     await page.waitForTimeout(200);
     assert.ok(await page.locator('.calculator-history').isVisible(),'wide: the history shows inline');
   }
 }
 if(name==='updates') await page.locator('.updates-details').waitFor();
 if(name==='browser' && phone) {
  // Phone: the address row is the only header row, and the tab count in it
  // opens the switcher as a pane over the page (left open to be measured;
  // Escape pops it below).
  assert.equal(await page.locator('.browser-tile > .tile-header--merged > .browser-bar').count(),1,'the address row is the window bar');
  assert.ok(!(await page.locator('.browser-tabstrip').evaluate(e=>e.getClientRects().length>0)),'the tab strip is hidden');
  const bar=await page.locator('.browser-bar').evaluate(e=>({width:e.clientWidth,scrollWidth:e.scrollWidth}));
  assert.ok(bar.scrollWidth<=bar.width+1,`the address row fits on one screen (${JSON.stringify(bar)})`);
  if(process.env.SHINY_UI_SCREENSHOTS) await page.screenshot({path:`${process.env.SHINY_UI_SCREENSHOTS}/${name}-${size.width}x${size.height}-${theme}-bar.png`});
  await page.locator('.browser-tabcount--bar').tap();
  await page.waitForTimeout(400);
  assert.ok(await page.locator('.browser-tabs-panel').evaluate(e=>e.classList.contains('is-open')),'the tab count opens the switcher pane');
  assert.ok(await page.locator('.browser-tabs-panel .ui-pane-back').evaluate(e=>e.getClientRects().length>0),'the switcher shows the shared Back');
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
   // A word split across two lines (letter by letter in a narrow box) reads
   // as broken; only a box that hyphenates on purpose may do it.
   const broken=[];
   const walker=document.createTreeWalker(root,NodeFilter.SHOW_TEXT);
   for(let n=walker.nextNode();n;n=walker.nextNode()){
     const p=n.parentElement;
     if(!p||!p.getClientRects().length||p.closest('textarea,input,pre,code,.xterm,[hidden]')||getComputedStyle(p).hyphens==='auto')continue;
     for(const m of n.data.matchAll(/\p{L}{2,}/gu)){
       // Part of an address, a path or a link: that has to break somewhere.
       const run=n.data.slice(0,m.index).split(/\s/).pop()+n.data.slice(m.index).split(/\s/)[0];
       if(/[@/.:_\\-]/.test(run))continue;
       const r=document.createRange();r.setStart(n,m.index);r.setEnd(n,m.index+m[0].length);
       const rects=[...r.getClientRects()].filter(x=>x.width>0);
       if(rects.length>1&&Math.abs(rects[0].top-rects[rects.length-1].top)>2)broken.push(`${m[0]} <${p.className||p.tagName}>`);
     }
   }
   // Phone: an action is an icon. A library button showing text (outside the
   // "More" menu, and not marked `data-text-ok`) is a regression, and so is a
   // window title bar or its buttons still on screen.
   const phone=document.documentElement.dataset.phone==='1';
   const textButtons=!phone?[]:[...root.querySelectorAll('.ui-btn')].filter(e=>{
     if(!e.getClientRects().length||getComputedStyle(e).visibility==='hidden'||e.closest('.ui-overflow-list,[data-text-ok],[hidden]'))return false;
     if(e.classList.contains('ui-btn--icon'))return false;
     return [...e.childNodes].some(n=>{
       if(n.nodeType===3)return n.textContent.trim().length>0;
       if(n.nodeType!==1||n.classList.contains('ui-icon')||n.classList.contains('ui-spinner'))return false;
       return n.getClientRects().length>0&&n.textContent.trim().length>0;
     });
   }).slice(0,15).map(e=>`${e.className} "${e.textContent.trim().slice(0,30)}"`);
   const titleBar=!phone?[]:[...root.querySelectorAll(':scope > .tile-header:not(.tile-header--merged), :scope > .tile-header > .tile-header-controls')].filter(e=>e.getClientRects().length&&getComputedStyle(e).display!=='none').map(e=>e.className);
   return {over,bars,clipped,broken:[...new Set(broken)].slice(0,12),textButtons,titleBar};
 });
 console.log(JSON.stringify({name,size,errors,...report}));
 if(errors.length || report.over.length || report.clipped.length || report.broken.length || fonts.length || report.textButtons.length || report.titleBar.length) failures.push({name,size,errors,over:report.over,clipped:report.clipped,broken:report.broken,fonts,textButtons:report.textButtons,titleBar:report.titleBar});
 if(process.env.SHINY_UI_SCREENSHOTS) await page.screenshot({path:`${process.env.SHINY_UI_SCREENSHOTS}/${name}-${size.width}x${size.height}${size.tileWidth ? `-tile${size.tileWidth}` : ''}-${theme}.png`});
 if(name==='settings') {
  for(const id of ['account','appearance','desktop','assistant','voice','images','music','touchbar','remote','system']) {
   await page.evaluate(id=>window.testModule.selectSettingsSection(id),id);
   const overflow=await page.evaluate(()=>{const p=document.querySelector('.settings-panels');return p.scrollWidth>p.clientWidth+2;});
   if(overflow) failures.push({name,size,section:id,overflow});
   // Phone: picking a section opens the stage pane, and every section's
   // actions are icons (the same rule as the window-level check above).
   if(phone) {
    const open=await page.evaluate(()=>document.querySelector('.settings-main')?.classList.contains('is-open'));
    if(!open) failures.push({name,size,section:id,failed:'the section did not open its pane'});
    const textButtons=await page.evaluate(()=>[...document.querySelectorAll('.settings-main .ui-btn')].filter(e=>{
     if(!e.getClientRects().length||getComputedStyle(e).visibility==='hidden'||e.closest('.ui-overflow-list,[data-text-ok],[hidden]'))return false;
     if(e.classList.contains('ui-btn--icon'))return false;
     return [...e.childNodes].some(n=>n.nodeType===3?n.textContent.trim().length>0:(n.nodeType===1&&!n.classList.contains('ui-icon')&&!n.classList.contains('ui-spinner')&&n.getClientRects().length>0&&n.textContent.trim().length>0));
    }).map(e=>`${e.className} "${e.textContent.trim().slice(0,30)}"`));
    if(textButtons.length) failures.push({name,size,section:id,textButtons});
    if(process.env.SHINY_UI_SCREENSHOTS) { await page.waitForTimeout(1200); await page.screenshot({path:`${process.env.SHINY_UI_SCREENSHOTS}/${name}-${size.width}x${size.height}-${id}-${theme}.png`}); }
   }
  }
  if(phone) {
   // The manifest is a third pane over the stage, from the bar's info button.
   await page.locator('.settings-main .settings-pane-bar .ui-btn--icon:not(.ui-pane-back)').tap();
   await page.waitForTimeout(400);
   assert.equal(await page.evaluate(()=>document.querySelector('.settings-manifest').classList.contains('is-open')),true,'the info button opens the manifest pane');
   if(process.env.SHINY_UI_SCREENSHOTS) await page.screenshot({path:`${process.env.SHINY_UI_SCREENSHOTS}/${name}-${size.width}x${size.height}-manifest-${theme}.png`});
   await page.keyboard.press('Escape');
   assert.equal(await page.evaluate(()=>document.querySelector('.settings-manifest').classList.contains('is-open')),false,'Escape closes the manifest pane');
   await page.keyboard.press('Escape');
   assert.equal(await page.evaluate(()=>document.querySelector('.settings-main').classList.contains('is-open')),false,'Escape closes the section pane');
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
 if(name==='impress' && (phone || size.tileWidth)) {
  // Narrow: the inspector is a pane over the stage, from the bar's properties button; Escape slides it back out.
  const tap=async(loc)=>{ if(size.touch===false) await loc.click(); else await loc.tap(); };
  const pane=page.locator('.impress-inspector');
  assert.ok(await page.locator('.impress-body').evaluate(e=>e.classList.contains('is-stacked')),'the body is stacked when narrow or on a phone');
  assert.ok(!(await pane.evaluate(e=>e.classList.contains('is-open'))),'the inspector pane starts closed');
  await tap(page.locator('.impress-inspector-btn'));
  await page.waitForTimeout(400);
  assert.ok(await pane.evaluate(e=>e.classList.contains('is-open')),'the properties button opens the inspector pane');
  const box=await pane.boundingBox();
  assert.ok(box && box.x>=0 && box.x+box.width<=size.width+1,'the inspector pane fits the viewport');
  assert.ok(await page.locator('.impress-inspector .ui-pane-back').evaluate(e=>e.getClientRects().length>0),'the inspector pane shows the shared Back');
  if(process.env.SHINY_UI_SCREENSHOTS) await page.screenshot({path:`${process.env.SHINY_UI_SCREENSHOTS}/${name}-${size.width}x${size.height}${size.tileWidth ? `-tile${size.tileWidth}` : ''}-${theme}-inspector.png`});
  await page.keyboard.press('Escape');
  await page.waitForTimeout(400);
  assert.ok(!(await pane.evaluate(e=>e.classList.contains('is-open'))),'Escape closes the inspector pane');
 }
 if(name==='calc' && phone) {
  // A finger has no double click: a second tap on the selected cell opens the
  // in-cell editor, Escape closes it.
  const cell=page.locator('.calc-cell--data[data-ref="B2"]');
  await cell.tap();
  assert.ok(await cell.evaluate(e=>e.classList.contains('is-selected')),'a tap selects the cell');
  assert.ok(!(await page.locator('.calc-cell-editor').evaluate(e=>e.getClientRects().length>0).catch(()=>false)),'one tap does not edit');
  await cell.tap();
  const editor=page.locator('.calc-cell-editor');
  await editor.waitFor({state:'visible'});
  assert.equal(await editor.inputValue(),'120','the second tap edits the cell in place');
  if(process.env.SHINY_UI_SCREENSHOTS) await page.screenshot({path:`${process.env.SHINY_UI_SCREENSHOTS}/${name}-${size.width}x${size.height}-${theme}-edit.png`});
  await page.keyboard.press('Escape');
  await editor.waitFor({state:'hidden'});
  // The bar's zoom tool (folded or not) scales the grid.
  const zoomed=await page.evaluate(()=>{document.querySelector('.calc-zoom-in').click();return getComputedStyle(document.querySelector('.calc-grid')).getPropertyValue('--calc-zoom').trim();});
  assert.equal(zoomed,'1.2','zoom in scales the grid');
  await page.evaluate(()=>document.querySelector('.calc-zoom-out').click());
 }
 if(name==='pdf' && (phone || size.tileWidth)) {
  // Narrow: the comments are a pane over the page, from the bar's button; Escape slides it out.
  const tap=async(loc)=>{ if(size.touch===false) await loc.click(); else await loc.tap(); };
  await page.locator('.pdf-page-canvas').waitFor();
  const pane=page.locator('.pdf-panel');
  assert.ok(await page.locator('.pdf-main').evaluate(e=>e.classList.contains('is-stacked')),'the PDF body is a pane stack');
  assert.ok(!(await pane.evaluate(e=>e.classList.contains('is-open'))),'the comments pane starts closed');
  await tap(page.locator('[data-window-bar] .pdf-tool--panel'));
  await page.waitForTimeout(400);
  assert.ok(await pane.evaluate(e=>e.classList.contains('is-open')),'the comments button opens the comments pane');
  const box=await pane.boundingBox();
  assert.ok(box && box.x>=0 && box.x+box.width<=size.width+1,'the comments pane fits the viewport');
  assert.ok(await page.locator('.pdf-panel .ui-pane-back').evaluate(e=>e.getClientRects().length>0),'the comments pane shows the shared Back');
  assert.ok(await page.locator('.pdf-comment').count()>=1,'the comments pane lists the comment');
  if(process.env.SHINY_UI_SCREENSHOTS) await page.screenshot({path:`${process.env.SHINY_UI_SCREENSHOTS}/${name}-${size.width}x${size.height}${size.tileWidth ? `-tile${size.tileWidth}` : ''}-${theme}-comments.png`});
  await page.keyboard.press('Escape');
  await page.waitForTimeout(400);
  assert.ok(!(await pane.evaluate(e=>e.classList.contains('is-open'))),'Escape closes the comments pane');
  assert.equal(await page.locator('[data-window-bar] .pdf-tool--panel').getAttribute('aria-pressed'),'false','closing the pane releases the comments button');
 }
 if(name==='calendar' && (phone || size.tileWidth)) {
  // Narrow: a day opens as a pane over the month; Escape slides it back out.
  const tap=async(loc)=>{ if(size.touch===false) await loc.click(); else await loc.tap(); };
  const pane=page.locator('.calendar-detail');
  assert.ok(!(await pane.evaluate(e=>e.classList.contains('is-open'))),'the day pane starts closed');
  await tap(page.locator('.calendar-day:not(.is-other)').first());
  await page.waitForTimeout(400);
  assert.ok(await pane.evaluate(e=>e.classList.contains('is-open')),'tapping a day opens the day pane');
  const box=await pane.boundingBox();
  assert.ok(box && box.x>=0 && box.x+box.width<=size.width+1,'the day pane fits the viewport');
  assert.ok(await page.locator('.calendar-detail .ui-pane-back').evaluate(e=>e.getClientRects().length>0),'the day pane shows the shared Back');
  if(process.env.SHINY_UI_SCREENSHOTS) await page.screenshot({path:`${process.env.SHINY_UI_SCREENSHOTS}/${name}-${size.width}x${size.height}${size.tileWidth ? `-tile${size.tileWidth}` : ''}-${theme}-day.png`});
  await page.keyboard.press('Escape');
  await page.waitForTimeout(400);
  assert.ok(!(await pane.evaluate(e=>e.classList.contains('is-open'))),'Escape closes the day pane');
 }
 if(name==='updates') {
  // Details opens the job log as a pane: narrow or phone it slides over the
  // content with the shared Back and Escape pops it; wide it is a column
  // beside the content that the same button hides again.
  const tap=async(loc)=>{ if(size.touch===false) await loc.click(); else await loc.tap(); };
  const pane=page.locator('.updates-log-pane');
  const stacked=await page.locator('.updates-shell').evaluate(e=>e.classList.contains('is-stacked'));
  assert.equal(stacked,!!(phone || size.tileWidth),'the log is stacked exactly when narrow or on a phone');
  await tap(page.locator('.updates-details'));
  await page.waitForTimeout(400);
  assert.ok(await pane.evaluate(e=>e.classList.contains('is-open')),'Details opens the log pane');
  const box=await pane.boundingBox();
  assert.ok(box && box.x>=0 && box.x+box.width<=size.width+1,'the log pane fits the viewport');
  if(stacked) assert.ok(await page.locator('.updates-log-pane .ui-pane-back').evaluate(e=>e.getClientRects().length>0),'the log pane shows the shared Back');
  if(process.env.SHINY_UI_SCREENSHOTS) await page.screenshot({path:`${process.env.SHINY_UI_SCREENSHOTS}/${name}-${size.width}x${size.height}${size.tileWidth ? `-tile${size.tileWidth}` : ''}-${theme}-log.png`});
  if(stacked) await page.keyboard.press('Escape'); else await tap(page.locator('.updates-details'));
  await page.waitForTimeout(400);
  assert.ok(!(await pane.evaluate(e=>e.classList.contains('is-open'))),'the log pane closes again');
 }
 if(name==='word' && (phone || size.tileWidth)) {
  // Narrow: the outline is a pane over the desk, from the bar's list button;
  // Escape slides it back out.
  const tap=async(loc)=>{ if(size.touch===false) await loc.click(); else await loc.tap(); };
  const pane=page.locator('.word-nav');
  assert.ok(!(await pane.evaluate(e=>e.classList.contains('is-open'))),'the outline pane starts closed');
  await tap(page.locator('.word-outline-btn'));
  await page.waitForTimeout(400);
  assert.ok(await pane.evaluate(e=>e.classList.contains('is-open')),'the outline button opens the outline pane');
  const box=await pane.boundingBox();
  assert.ok(box && box.x>=0 && box.x+box.width<=size.width+1,'the outline pane fits the viewport');
  assert.ok(await page.locator('.word-nav .ui-pane-back').evaluate(e=>e.getClientRects().length>0),'the outline pane shows the shared Back');
  if(process.env.SHINY_UI_SCREENSHOTS) await page.screenshot({path:`${process.env.SHINY_UI_SCREENSHOTS}/${name}-${size.width}x${size.height}${size.tileWidth ? `-tile${size.tileWidth}` : ''}-${theme}-outline.png`});
  // Picking a heading pops the pane and lands the caret on it.
  await tap(page.locator('.word-nav-item').first());
  await page.waitForTimeout(400);
  assert.ok(!(await pane.evaluate(e=>e.classList.contains('is-open'))),'picking a heading closes the outline pane');
  if(phone) {
   // Phone: the formatting row is a bottom sheet behind the bar's Formatting
   // button; a tool acts on the editor's selection and closes the sheet.
   assert.ok(!(await page.locator('.word-format-btn').evaluate(e=>e.hidden)),'the phone bar shows the Formatting button');
   assert.ok(!(await page.locator('.word-tile .word-format').count()),'the formatting row left the window');
   await page.evaluate(()=>{const ed=document.querySelector('.word-content');const p=ed.querySelector('p');const r=document.createRange();r.selectNodeContents(p);const s=getSelection();s.removeAllRanges();s.addRange(r);ed.focus();});
   await tap(page.locator('.word-format-btn'));
   const sheetEl=page.locator('.word-format-sheet.is-open');
   await sheetEl.waitFor({state:'visible'});
   const card=await page.locator('.word-format-sheet .ui-sheet-card').boundingBox();
   assert.ok(card && card.x>=0 && card.x+card.width<=size.width+1 && card.y+card.height<=size.height+1,'the formatting sheet fits the viewport');
   const sheetText=await page.evaluate(()=>[...document.querySelectorAll('.word-format-sheet .ui-btn')].filter(e=>e.getClientRects().length&&!e.classList.contains('ui-btn--icon')&&[...e.childNodes].some(n=>n.nodeType===3?n.textContent.trim():(n.nodeType===1&&!n.classList.contains('ui-icon')&&n.getClientRects().length&&n.textContent.trim()))).map(e=>e.className));
   assert.deepEqual(sheetText,[],'the formatting sheet shows no text button');
   if(process.env.SHINY_UI_SCREENSHOTS) await page.screenshot({path:`${process.env.SHINY_UI_SCREENSHOTS}/${name}-${size.width}x${size.height}-${theme}-format.png`});
   await tap(page.locator('.word-format-sheet .word-cluster--emphasis .word-tool').first());
   await page.waitForTimeout(400);
   assert.equal(await page.locator('.word-format-sheet.is-open').count(),0,'a tool closes the formatting sheet');
   assert.ok(await page.evaluate(()=>!!document.querySelector('.word-content p').querySelector('b,strong,[style*="font-weight"]')),'Bold from the sheet applies to the selection');
   await tap(page.locator('.word-format-btn'));
   await page.locator('.word-format-sheet.is-open').waitFor({state:'visible'});
   await page.keyboard.press('Escape');
   await page.waitForTimeout(400);
   assert.equal(await page.locator('.word-format-sheet.is-open').count(),0,'Escape closes the formatting sheet');
  }
 }
 if(name==='studio') {
  // Narrow or phone: the detail panel, the browser and the AI panel are panes
  // over the body, opened from the bar and closed by Escape.
  const tap=async(loc)=>{ if(size.touch===false) await loc.click(); else await loc.tap(); };
  const stacked=await page.locator('.studio-shell').evaluate(e=>e.classList.contains('is-stacked'));
  assert.equal(stacked,!!(phone || size.tileWidth),'the shell is stacked exactly when narrow or on a phone');
  if(stacked) {
   const isOpen=(sel)=>page.locator(sel).evaluate(e=>e.classList.contains('is-open'));
   await tap(page.locator('.studio-detail-btn'));
   await page.waitForTimeout(400);
   assert.ok(await isOpen('.studio-detail'),'the bar button opens the detail pane');
   const box=await page.locator('.studio-detail').boundingBox();
   assert.ok(box && box.x>=0 && box.x+box.width<=size.width+1,'the detail pane fits the viewport');
   assert.ok(await page.locator('.studio-detail .ui-pane-back').evaluate(e=>e.getClientRects().length>0),'the detail pane shows the shared Back');
   if(process.env.SHINY_UI_SCREENSHOTS) await page.screenshot({path:`${process.env.SHINY_UI_SCREENSHOTS}/${name}-${size.width}x${size.height}${size.tileWidth ? `-tile${size.tileWidth}` : ''}-${theme}-detail.png`});
   await page.keyboard.press('Escape');
   await page.waitForTimeout(400);
   assert.ok(!(await isOpen('.studio-detail')),'Escape closes the detail pane');
   await tap(page.locator('.studio-menu-btn'));
   await page.waitForTimeout(400);
   assert.ok(await isOpen('.studio-browser:not(.studio-ai)'),'the Browser button opens the browser pane');
   if(process.env.SHINY_UI_SCREENSHOTS) await page.screenshot({path:`${process.env.SHINY_UI_SCREENSHOTS}/${name}-${size.width}x${size.height}${size.tileWidth ? `-tile${size.tileWidth}` : ''}-${theme}-browser.png`});
   await page.keyboard.press('Escape');
   await page.waitForTimeout(400);
   assert.ok(!(await isOpen('.studio-browser:not(.studio-ai)')),'Escape closes the browser pane');
  }
 }
 if(name==='browser' && phone) {
  await page.keyboard.press('Escape');
  await page.waitForTimeout(400);
  assert.ok(!(await page.locator('.browser-tabs-panel').evaluate(e=>e.classList.contains('is-open'))),'Escape closes the switcher pane');
  // History, from the menu, is a pane the same way.
  await page.locator('.browser-menu-btn').tap();
  await page.locator('.browser-menu-item', { hasText: 'History' }).tap();
  await page.waitForTimeout(400);
  assert.ok(await page.locator('.browser-history').evaluate(e=>e.classList.contains('is-open')),'the menu opens history as a pane');
  if(process.env.SHINY_UI_SCREENSHOTS) await page.screenshot({path:`${process.env.SHINY_UI_SCREENSHOTS}/${name}-${size.width}x${size.height}-${theme}-history.png`});
  await page.keyboard.press('Escape');
  await page.waitForTimeout(400);
  assert.ok(!(await page.locator('.browser-history').evaluate(e=>e.classList.contains('is-open'))),'Escape closes the history pane');
 }
 if(name==='plugins') {
  // The window mode lives in the manifest of an installed plugin with a window.
  const tap=async(loc)=>{ if(size.touch===false) await loc.click(); else await loc.tap(); };
  await tap(page.locator('.pw-rail-item[data-section="installed"]'));
  await tap(page.locator('.pw-row').first());
  await page.waitForTimeout(400);
  const select=page.locator('.pw-manifest .ui-select').first();
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
