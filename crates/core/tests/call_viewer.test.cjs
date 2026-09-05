// Run with node --test crates/core/tests/call_viewer.test.cjs.
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
const html=fs.readFileSync(`${__dirname}/../src/call_viewer.html`,'utf8');
const script=html.slice(html.indexOf('    const cppKeywords='),html.indexOf('    window.showOccurrence='));
function render(lines,evidence,status='inferred',startColumn=1){
  const context={TextEncoder,outgoing:new Map([['caller',[{target:'callee',status,evidence}]]]),nodes:new Map([['callee',{name:'stock_for'}]]),statusLabels:{inferred:'Likely match',exact:'Exact match',unresolved:'Unresolved'},esc:s=>String(s).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]))};
  vm.createContext(context);vm.runInContext(script,context);
  return context.sourceHtml({start_line:7,start_column:startColumn,end_line:6+lines.length,lines},[],'caller','a.cpp');
}
const ev=(expression,column=1,line=7,path='a.cpp',callee='stock_for')=>({expression,column,line,path,callee});
test('marks only the recorded call, retaining syntax, escaping and uncertainty',()=>{
 const result=render(['if (stock_for(x) < 2) stock_for(y); // stock_for(z)'],[ev('stock_for(x)',5)]);
 assert.equal((result.match(/<mark /g)||[]).length,1);
 assert.match(result,/status-inferred/);assert.match(result,/Likely match/);assert.match(result,/tok-keyword/);assert.match(result,/&lt;/ );
 assert.match(result,/tok-comment/);
});
test('handles repeated, nested and multiline calls by their own positions',()=>{
 const result=render(['stock_for(stock_for(x));','stock_for(','x);'],[ev('stock_for(stock_for(x))'),ev('stock_for(x)',11),ev('stock_for(\nx)',1,8)]);
 assert.equal((result.match(/<mark /g)||[]).length,3);
});
test('does not mark wrong paths, stale evidence, strings or comments',()=>{
 for(const [line,evidence] of [['stock_for(x)',ev('stock_for(x)',1,7,'b.cpp')],['stock_for(y)',ev('stock_for(x)')],['"stock_for(x)"',ev('stock_for(x)',2)],['// stock_for(x)',ev('stock_for(x)',4)]])assert.doesNotMatch(render([line],[evidence]),/<mark /);
});
test('handles UTF-8 columns and excerpts starting partway through a line',()=>{
 assert.match(render(['/*é*/ stock_for(x)'],[ev('stock_for(x)',8)]),/<mark /);
 assert.match(render(['stock_for(x)'],[ev('stock_for(x)',12)],'exact',12),/status-exact/);
 assert.match(render(['stock_for(x)'],[ev('stock_for(x)')],'unresolved'),/status-unresolved/);
});
test('qualified template calls keep token styling and escape angle brackets',()=>{
 const result=render(['ns::stock_for<int>(x)'],[ev('ns::stock_for<int>(x)',1,7,'a.cpp','stock_for<int>')]);
 assert.match(result,/<mark [^>]*>stock_for<\/mark>&lt;<span class="tok-type">int<\/span>&gt;/);
});

function layoutContext(){
  const context={groupedSequence:false,document:{getElementById:()=>({})},orientationOffsets:{horizontal:new Map(),vertical:new Map()},viewport:{x:0,y:0,scale:1},renderGraph:()=>{}};
  vm.createContext(context);
  vm.runInContext(html.slice(html.indexOf('    function layoutPosition('),html.indexOf('    function renderGraph(')),context);
  vm.runInContext(html.slice(html.indexOf('    function routeEdge('),html.indexOf('    function edgePath(')),context);
  return context;
}
test('orientation places callers before focus and callees after at every depth without overlaps',()=>{
 const c=layoutContext();
 for(const vertical of [false,true])for(const depth of [-3,-2,-1,0,1,2,3]){
  const points=Array.from({length:7},(_,i)=>c.layoutPosition(depth,i,7,500,400,vertical));
  for(let i=0;i<points.length;i++){
   assert.equal(vertical?points[i].y:points[i].x,(vertical?400:500)+depth*(vertical?190:330));
   if(i)assert.ok((vertical?points[i].x-points[i-1].x:points[i].y-points[i-1].y)>=(vertical?220:76));
  }
 }
});
test('edge ports rotate from right/left to bottom/top including backward calls',()=>{
 const c=layoutContext(),from={x:500,y:400};
 assert.match(c.routeEdge(from,{x:830,y:400},220,58,false),/^M 610 400 .*720 400$/);
 assert.match(c.routeEdge(from,{x:500,y:590},220,58,true),/^M 500 429 .*500 561$/);
 assert.match(c.routeEdge(from,{x:500,y:210},220,58,true),/^M 500 371 .*500 239$/);
 assert.doesNotMatch(c.routeEdge(from,{x:750,y:400},220,76,true),/NaN|undefined/);
 assert.doesNotMatch(c.routeEdge(from,from,220,76,true,true),/NaN|undefined/);
});
test('switching orientation preserves independent dragged positions and open panels',()=>{
 const c=layoutContext();let refresh;
 c.renderGraph=value=>{refresh=value};c.recenterGraph=()=>{};
 c.orientationOffsets.horizontal.set('a',{x:20,y:10});c.orientationOffsets.vertical.set('a',{x:-40,y:80});
 c.setOrientation(true);assert.equal(c.nodeOffsets.get('a').x,-40);assert.equal(refresh,false);
 c.nodeOffsets.clear();c.setOrientation(false);assert.equal(c.nodeOffsets.get('a').x,20);assert.equal(c.viewport.scale,1);
});
test('top-down recenter fits wide rows without discarding drag offsets',()=>{
 const c=layoutContext();c.topDown=true;c.nodeW=220;c.nodeH=76;
 c.graphPositions=new Map([['a',{x:-250,y:100}],['b',{x:1250,y:500}]]);
 c.document={getElementById:()=>({viewBox:{baseVal:{width:1000,height:600}}})};c.applyViewport=()=>{};
 c.recenterGraph();assert.ok(c.viewport.scale<1);
 for(const p of c.graphPositions.values()){
  assert.ok((p.x-110)*c.viewport.scale+c.viewport.x>=0);
  assert.ok((p.x+110)*c.viewport.scale+c.viewport.x<=1000);
 }
 assert.equal(c.graphPositions.get('a').x,-250);
});

function groupedContext(){
 const c=layoutContext();c.nodeW=220;c.nodeH=76;c.nodeOffsets=new Map();
 c.nodes=new Map(['s','a','b','c','d'].map(id=>[id,{id,name:id,short_name:id,path:'a.cpp'}]));
 c.outgoing=new Map();
 vm.runInContext(html.slice(html.indexOf('    function sourceSites('),html.indexOf('    function layoutPosition(')),c);
 return c;
}
test('native groups stack normal function identities in source order and separate callers',()=>{
 const c=groupedContext();
 c.outgoing.set('s',[{target:'a',evidence:[{path:'a.cpp',line:10,column:1},{path:'a.cpp',line:20,column:1}]},{target:'b',evidence:[{path:'a.cpp',line:2,column:1}]}]);
 const layer=new Map([['s',0],['a',1],['b',1],['c',2],['d',2]]),edges=[{source:'s',target:'a'},{source:'s',target:'b'},{source:'a',target:'c'},{source:'b',target:'d'}];
 const result=c.groupedPositions(layer,edges,500,300);
 assert.equal(result.positions.size,5);assert.ok(result.positions.get('b').y<result.positions.get('a').y);
 assert.equal(result.positions.get('a').x,result.positions.get('b').x);
 assert.match(result.info.get('a').text,/2 call sites/);
 assert.equal(result.groups.filter(g=>g.owners[0]==='a').length,1);assert.equal(result.groups.filter(g=>g.owners[0]==='b').length,1);
 assert.ok(result.positions.get('c').y>result.positions.get('a').y+76);
});
test('shared callees have one identity and explicitly no shared source order',()=>{
 const c=groupedContext(),result=c.groupedPositions(new Map([['s',0],['a',1],['b',1],['c',2]]),[{source:'s',target:'a'},{source:'s',target:'b'},{source:'a',target:'c'},{source:'b',target:'c'}],500,300);
 assert.equal(result.positions.size,4);assert.equal(result.groups.at(-1).label,'Shared callees');
 assert.match(result.groups.at(-1).caption,/no shared order/);
});
test('compact group cues derive from parser ancestry',()=>{
 const c=groupedContext(),call={kind:'call',line:7,column:9,children:[]};
 c.nodes.get('s').call_flow={kind:'loop',children:[{kind:'condition',children:[call]}]};
 assert.equal(c.callCue('s',{line:7,column:9}).join(', '),'loop, condition');
 assert.equal(c.callCue('s',{line:8,column:9}).length,0);
});
