const {test}=require('node:test');
const assert=require('node:assert/strict');
const runtime=require('../src/explorer_runtime.js');
const languages=require('../src/explorer_languages.js');
function element(){return {handlers:{},attrs:{},value:'1',min:'0',max:'20',addEventListener(n,f){this.handlers[n]=f},setAttribute(n,v){this.attrs[n]=v},setPointerCapture(id){this.pointer=id},hasPointerCapture(id){return this.pointer===id},releasePointerCapture(){this.pointer=null}}}
test('shared depth clamps invalid input and synchronizes both controls',()=>{
 const slider=element(),input=element(),seen=[];runtime.bindDepth(slider,input,x=>seen.push(x));
 slider.value='99';slider.handlers.input();assert.equal(input.value,20);
 input.value='no';input.handlers.change();assert.equal(slider.value,0);
 input.value='3';input.handlers.keydown({key:'Enter',preventDefault(){}});assert.deepEqual(seen,[20,0,3]);
});
test('shared resize tracks one pointer, cancels inspection, and cleans up lost capture',()=>{
 global.document={body:{classList:{add(){},remove(){}}}};
 const handle=element();let width=260,cancelled=0;
 runtime.bindResizer(handle,{read:()=>width,write:x=>width=x,initial:260,sign:-1,cancelInspection:()=>cancelled++});
 handle.handlers.pointerdown({button:0,pointerId:1,clientX:300,preventDefault(){}});
 handle.handlers.pointermove({pointerId:2,clientX:100});assert.equal(width,260);
 handle.handlers.pointermove({pointerId:1,clientX:200});assert.equal(width,360);
 handle.handlers.lostpointercapture({pointerId:1});handle.handlers.pointermove({pointerId:1,clientX:100});assert.equal(width,360);
 handle.handlers.keydown({key:'Home',preventDefault(){}});assert.equal(width,260);assert.equal(cancelled,2);
 delete global.document;
});
test('Python highlighting uses Python comments and retains multiline docstrings',()=>{
 const highlight=languages.highlighter('python'),state={};
 assert.match(highlight('def work(): # hello',state),/tok-keyword.*tok-comment/);
 assert.doesNotMatch(highlight('# comment',state),/tok-preproc/);
 assert.match(highlight('"""first',state),/tok-string/);
 assert.match(highlight('def inside docstring',state),/tok-string/);
 assert.doesNotMatch(highlight('# text inside docstring',state),/tok-comment/);
 assert.doesNotMatch(highlight('def inside docstring',state),/tok-keyword/);
 highlight('last"""',state);assert.match(highlight('return 1',state),/tok-keyword/);
});
test('unknown language source stays plain and escapes markup without C++ guesses',()=>{
 const highlight=languages.highlighter('future-language');
 assert.equal(highlight('class <script>& "value"',{}),'class &lt;script&gt;&amp; &quot;value&quot;');
 assert.equal(languages.owner('X::method','future-language'),'Unspecified scope');
 assert.equal(languages.owner('shop.service::Store.run','python'),'shop.service.Store');
 assert.equal(languages.owner('parcel::Store::run','cpp'),'parcel::Store');
});

test('containment follows language separators and never guesses unknown scopes',()=>{
 for(const [language,name,child] of [['python','shop::Client','shop::Client.run'],['cpp','parcel::Client','parcel::Client::run']]){
  assert(languages.isDirectChild({language,name},{language,name:child}));
 }
 assert(!languages.isDirectChild({language:'future',name:'X'},{language:'future',name:'X::run'}));
});
