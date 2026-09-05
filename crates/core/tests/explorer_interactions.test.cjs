const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
function fixture(){
 const timers=new Map();let next=0,details=0,explored=0,blocked=false;
 class Element {
  constructor(){this.handlers={};this.children=[];this.style={};this.offsetWidth=150;this.offsetHeight=70}
  addEventListener(name,fn){(this.handlers[name]??=[]).push(fn)}
  emit(name,values={}){const event={detail:1,preventDefault(){},stopPropagation(){},...values};for(const fn of this.handlers[name]||[])fn(event);this['on'+name]?.(event)}
  append(child){child.parent=this;this.children.push(child)}
  remove(){this.parent.children=this.parent.children.filter(e=>e!==this)}
  setAttribute(){} getBoundingClientRect(){return {left:20,top:30}}
  focus(){doc.activeElement=this} contains(e){return e===this||this.children.includes(e)}
  get firstElementChild(){return this.children[0]}
 }
 const doc=new Element();doc.body=new Element();doc.createElement=()=>new Element();
 const context={document:doc,window:new Element(),innerWidth:800,innerHeight:600,setTimeout:fn=>{timers.set(++next,fn);return next},clearTimeout:id=>timers.delete(id)};
 vm.createContext(context);vm.runInContext(fs.readFileSync(`${__dirname}/../src/explorer_interactions.js`,'utf8')+'\nthis.interactions=explorerInteractions;',context);
 const node=new Element();context.interactions.bind(node,{inspect:()=>details++,explore:()=>explored++,blocked:()=>blocked});
 return {node,doc,flush(){const pending=[...timers.values()];timers.clear();pending.forEach(fn=>fn())},counts:()=>[details,explored],block:()=>blocked=true};
}
test('single click inspects; native double click explores without opening details first',()=>{
 const f=fixture();f.node.emit('click');assert.deepEqual(f.counts(),[0,0]);f.flush();assert.deepEqual(f.counts(),[1,0]);
 f.node.emit('click');f.node.emit('click',{detail:2});f.node.emit('dblclick');f.flush();assert.deepEqual(f.counts(),[1,1]);
});
test('drag suppression blocks click and double click, and starting a drag cancels pending inspection',()=>{
 const f=fixture();f.node.emit('click');f.node.emit('pointerdown');f.block();f.node.emit('click');f.node.emit('dblclick');f.flush();assert.deepEqual(f.counts(),[0,0]);
});
test('right click offers explicit details and explore, Escape dismisses and restores focus',()=>{
 const f=fixture();f.node.emit('contextmenu');let menu=f.doc.body.children[0];assert.deepEqual(menu.children.map(e=>e.textContent),['Show details','Explore']);menu.children[1].emit('click');assert.deepEqual(f.counts(),[0,1]);assert.equal(f.doc.body.children.length,0);
 f.node.emit('contextmenu');menu=f.doc.body.children[0];menu.emit('keydown',{key:'Escape'});assert.equal(f.doc.activeElement,f.node);assert.deepEqual(f.counts(),[0,1]);
 f.node.emit('contextmenu');f.doc.body.children[0].children[0].emit('click');assert.deepEqual(f.counts(),[1,1]);
});
test('keyboard inspection and exploration stay separate; keyboard context menu works',()=>{
 const f=fixture();f.node.emit('keydown',{key:'Enter'});f.node.emit('keydown',{key:' '});f.node.emit('keydown',{key:'Enter',shiftKey:true});assert.deepEqual(f.counts(),[2,1]);f.node.emit('keydown',{key:'F10',shiftKey:true});assert.equal(f.doc.body.children.length,1);
});

test('starting a background gesture cancels a pending node inspection',()=>{
 const f=fixture();f.node.emit('click');f.doc.emit('pointerdown',{target:f.doc.body});f.flush();assert.deepEqual(f.counts(),[0,0]);
});
