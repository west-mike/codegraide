const {test}=require('node:test');
const assert=require('node:assert/strict');
const t=require('../src/dependency_explorer.js');
const nodes=[{id:'a',name:'domain/a.hpp',path:'domain/a.hpp',kind:'local-file'},{id:'b',name:'storage/b.hpp',path:'storage/b.hpp',kind:'local-file'},{id:'c',name:'storage/detail/c.hpp',path:'storage/detail/c.hpp',kind:'local-file'},{id:'x',name:'vector',kind:'system-header'}];
const relations=[{source:'a',target:'b',kind:'exact'},{source:'b',target:'c',kind:'exact'},{source:'c',target:'a',kind:'exact'},{source:'a',target:'x',kind:'inferred'}];
test('trace respects depth, direction, cycles and full reachability',()=>{
 assert.deepEqual([...t.trace(nodes,relations,['a'],'dependencies',0)],['a']);
 assert.deepEqual([...t.trace(nodes,relations,['a'],'dependencies',1)].sort(),['a','b','x']);
 assert.deepEqual([...t.trace(nodes,relations,['a'],'dependents',1)].sort(),['a','c']);
 assert.deepEqual([...t.trace(nodes,relations,['a'],'both',Infinity)].sort(),['a','b','c','x']);
});
test('path uses exact local dependencies, handles same node and disconnected nodes',()=>{
 assert.deepEqual(t.path(nodes,relations,'a','c'),['a','b','c']);
 assert.deepEqual(t.path(nodes,relations,'a','a'),['a']);
 assert.equal(t.path(nodes,relations,'a','x'),null);
 assert.equal(t.path(nodes,relations.filter(r=>r.target!=='c'),'a','c'),null);
 assert.equal(t.path(nodes,relations,'missing','c'),null);
});
const config={format:'dependency-architecture-v1',groups:[{name:'Domain',prefixes:['domain']},{name:'Storage',prefixes:['storage']},{name:'Detail',prefixes:['storage/detail']}],rules:[{from:'Domain',to:'Storage'}]};
test('architecture uses path boundaries and most specific prefix',()=>{
 const groups=t.membership([...nodes,{id:'d',name:'storage2/a.hpp',kind:'local-file'}],config);
 assert.equal(groups.get('a'),'Domain');assert.equal(groups.get('b'),'Storage');assert.equal(groups.get('c'),'Detail');assert.equal(groups.has('d'),false);assert.equal(groups.has('x'),false);
});
test('rule findings distinguish exact violations from uncertain matches',()=>{
 const findings=t.violations(nodes,[...relations,{source:'a',target:'b',kind:'inferred'}],config);
 assert.equal(findings.length,2);assert.equal(findings[0].confirmed,true);assert.equal(findings[1].confirmed,false);
});
test('architecture validation rejects ambiguous configuration and unsafe paths',()=>{
 assert.equal(t.validateArchitecture(config).groups.length,3);
 for(const invalid of [{...config,groups:[...config.groups,config.groups[0]]},{...config,rules:[{from:'Domain',to:'missing'}]},{...config,groups:[{name:'A',prefixes:['../x']}]},{...config,groups:[{name:'A',prefixes:['/x']}]},{...config,groups:[{name:'A',prefixes:['a']},{name:'B',prefixes:['a']}]}])assert.throws(()=>t.validateArchitecture(invalid));
});
test('comparison ignores regenerated IDs and reports status changes',()=>{
 const snapshot=t.snapshot({nodes,relations},'abc123');t.validateSnapshot(snapshot);
 const current={nodes:nodes.map(n=>({...n,id:n.id+'2'})),relations:relations.map(r=>({...r,source:r.source+'2',target:r.target+'2'}))};
 assert.deepEqual(t.compare(current,snapshot),{added:[],removed:[]});
 current.relations[0].kind='inferred';const diff=t.compare(current,snapshot);assert.equal(diff.added.length,1);assert.equal(diff.removed.length,1);assert.equal(diff.added[0].kind,'inferred');
});
test('comparison retains removed files and dependencies',()=>{
 const baseline=t.snapshot({nodes,relations},'older');const diff=t.compare({nodes:nodes.slice(0,2),relations:relations.slice(0,1)},baseline);
 assert.equal(diff.removed.length,3);assert.ok(diff.removed.some(r=>r.target.name==='storage/detail/c.hpp'));
});
test('snapshot validation rejects missing endpoints and duplicate identities',()=>{
 const snapshot=t.snapshot({nodes,relations},'head');
 assert.throws(()=>t.validateSnapshot({...snapshot,nodes:[]}));
 assert.throws(()=>t.validateSnapshot({...snapshot,nodes:[...nodes,{...nodes[0],id:'other'}]}));
 assert.throws(()=>t.validateSnapshot({nodes,relations}));
});

test('large graph window keeps focus and neighbours and never emits dangling edges',()=>{
 const graph={items:Array.from({length:300},(_,i)=>({id:String(i)})),relations:Array.from({length:299},(_,i)=>({source:String(i),target:String(i+1)}))};
 const result=t.graphWindow(graph,'200',5),ids=new Set(result.items.map(n=>n.id));
 assert.equal(result.omitted,295);assert(ids.has('200'));assert(ids.has('199'));assert(ids.has('201'));assert(result.relations.every(r=>ids.has(r.source)&&ids.has(r.target)));
 assert.equal(t.graphWindow(graph,'200',400).omitted,0);
});
