import './helpers/i18n.mjs';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { placePopover, placeMenu } from '../../ui/popover.js';
import { dial } from '../../ui/components/dial.js';

class Node extends EventTarget {
  constructor(tag) {
    super(); this.tagName=tag.toUpperCase(); this.children=[]; this.dataset={}; this.style={}; this.attributes={}; this.className='';
    this.classList={add: (...names)=>{this.className=[...new Set([...this.className.split(' ').filter(Boolean),...names])].join(' ');},toggle:(name,on)=>{const a=this.className.split(' ').filter(n=>n&&n!==name);if(on)a.push(name);this.className=a.join(' ');},remove: name=>{this.className=this.className.split(' ').filter(n=>n!==name).join(' ');}};
  }
  setAttribute(key,value) { this.attributes[key]=String(value); }
  append(...nodes) { this.children.push(...nodes); }
  appendChild(node) { this.append(node); return node; }
}
globalThis.document={createElement:tag=>new Node(tag),createElementNS:(_ns,tag)=>new Node(tag)};
globalThis.window={setTimeout,clearTimeout,innerWidth:320,innerHeight:360,visualViewport:{offsetLeft:0,offsetTop:0,width:320,height:220}};
const popup={style:{},get offsetWidth(){return Math.min(400,parseFloat(this.style.maxWidth));},get offsetHeight(){return Math.min(300,parseFloat(this.style.maxHeight));}};
placePopover(popup,{getBoundingClientRect:()=>({left:290,top:180,bottom:210,width:600})});
assert.equal(popup.style.maxWidth,'304px');
assert.equal(popup.style.minWidth,'304px','wide anchors cannot force popovers offscreen');
assert.equal(popup.style.left,'8px');
assert.ok(parseFloat(popup.style.top)+popup.offsetHeight<=212,'popup fits above a soft keyboard');

/* placeMenu keeps a chip's menu on screen: under the chip, or ABOVE it when the
   chip sits at the bottom of the screen — a top bar docked at the bottom edge
   would otherwise open every menu off-screen. */
globalThis.window.innerWidth=1000;
globalThis.window.innerHeight=700;
const chipMenu={style:{},offsetWidth:200,offsetHeight:260};
const chip=bottom=>({getBoundingClientRect:()=>({left:40,top:bottom-28,right:140,bottom})});
placeMenu(chipMenu,chip(688));
assert.equal(chipMenu.style.left,'40px');
assert.equal(chipMenu.style.top,'392px','a menu at the bottom edge opens upward');
assert.equal(chipMenu.style.maxHeight,'640px','the room above the chip caps it');
placeMenu(chipMenu,chip(40));
assert.equal(chipMenu.style.top,'48px','a menu at the top edge opens under its chip');
assert.equal(chipMenu.style.maxHeight,'640px');

/* …and every HUD chip menu goes through it rather than opening downward. */
for (const file of ['clockMenu.js','weatherMenu.js','networkMenu.js','bluetoothMenu.js',
  'batteryMenu.js','audioMenu.js','clipboardMenu.js','powerMenu.js']) {
  const src=readFileSync(new URL(`../${file}`,import.meta.url),'utf8');
  assert.ok(/placeMenu\(popup, trigger/.test(src),`${file} uses the shared menu placement`);
  assert.ok(!/rect\.bottom \+ 8/.test(src),`${file} does not open downward unconditionally`);
}
const preview=[],commits=[];
const knob=dial({label:'Brightness',min:-100,max:100,step:1,format:String,onInput:v=>preview.push(v),onChange:v=>commits.push(v)});
const emit=(type,props={})=>knob.dispatchEvent(Object.assign(new Event(type,{cancelable:true}),props));
assert.equal(knob.attributes['aria-label'],'Brightness');
emit('keydown',{key:'End'}); assert.equal(knob.value,100);
emit('keydown',{key:'ArrowUp'}); assert.equal(knob.value,100,'value clamps at max');
emit('keydown',{key:'Home'}); assert.equal(knob.value,-100);
emit('keydown',{key:'Delete'}); assert.equal(knob.value,0);
assert.deepEqual(preview,[],'discrete edits commit once without a redundant preview');
knob.setValue(25); assert.equal(commits.length,4,'automation updates are silent');
emit('pointerdown',{clientY:200,pointerId:1,button:0});
emit('pointermove',{clientY:160}); assert.equal(knob.value,75); assert.deepEqual(preview,[75]);
emit('pointerup',{pointerId:1}); assert.equal(commits.at(-1),75);
assert.equal(knob.attributes['aria-valuenow'],'75');
emit('wheel',{deltaY:-1}); const before=commits.length; knob.destroy();
await new Promise(r=>setTimeout(r,380));
assert.equal(commits.length,before,'unmount cancels pending wheel commits');
console.log('Shared dial input, accessibility, cleanup and popover viewport checks passed');
