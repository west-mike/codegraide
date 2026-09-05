const data=JSON.parse(document.getElementById("graph-data").textContent);
    const nodes=new Map(data.nodes.map(n=>[n.id,n]));
    const nameCounts=new Map();for(const node of data.nodes)if(node.path)nameCounts.set(node.name,(nameCounts.get(node.name)||0)+1);
    const displayIdentity=node=>nameCounts.get(node.name)>1&&node.path?node.name+' · '+node.path:node.name;
    const outgoing=new Map(),incoming=new Map();
    for(const r of data.relations){if(!outgoing.has(r.source))outgoing.set(r.source,[]);outgoing.get(r.source).push(r);if(!incoming.has(r.target))incoming.set(r.target,[]);incoming.get(r.target).push(r)}
    const homeSelection=data.initial_selection||data.nodes.find(n=>n.path&&!n.noise)?.id||data.nodes[0]?.id;
    let selected=homeSelection,inspected=homeSelection;
    let perspective=data.nodes.some(n=>n.primary_architecture_group)?'architecture':'symbols',activeGroup=null,detailTab='details',graphDepth=1,graphExtra=0,showUncertain=false,showSupport=false,showNoise=false,history=[],renderedCards=0,selectedOccurrence=0,groupLimit=12,leftOpen=true,rightOpen=false;
    let viewport={x:0,y:0,scale:1},drag=null,noticeTimer=null,suppressNodeClick=false;
    // Offsets survive neighborhood changes without tying nodes to a viewport size.
    const orientationOffsets={horizontal:new Map(),vertical:new Map(),grouped:new Map()};
    let groupedSequence=false;let graphGroups=[],groupedNodeInfo=new Map(),groupRoutes=[];
    let topDown=false,nodeOffsets=orientationOffsets.horizontal;
    let graphPositions=new Map(),graphEdges=[];
    const nodeW=220;let nodeH=58,showFileNames=false;
    const esc=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
    const statusLabels={exact:'Exact match',inferred:'Likely match',ambiguous:'Several possible matches',external:'Outside this project',unresolved:'No match found',unavailable:'Could not analyze'};
    const perspectives=[['files','Files'],['symbols','Symbols'],...(data.nodes.some(n=>n.module)?[['modules','Modules']]:[]),...(data.nodes.some(n=>n.architecture_groups.length)?[['architecture','Architecture']]:[])];
    document.getElementById('sequenceToggle').parentElement.hidden=!data.nodes.some(n=>n.call_flow);
    const detailTabs=[['details','Overview'],['source','Code'],['calls','Calls']];
    document.getElementById('summary').textContent=`${data.nodes.filter(n=>n.path).length} symbols · ${data.relations.reduce((sum,r)=>sum+Math.max(r.evidence.length,1),0 )} call sites${data.filtered_report?' · filtered report':''}`;
    const coverageCounts=new Map();for(const r of data.relations)coverageCounts.set(r.status,(coverageCounts.get(r.status)||0)+Math.max(r.evidence.length,1));
    const resolutionLabels={exact:'Exact',inferred:'Inferred',ambiguous:'Ambiguous',external:'External',unresolved:'Unresolved',unavailable:'Not analyzed'};
    document.getElementById('analysisCoverage').innerHTML=[...coverageCounts].map(([status,count])=>`<div>${esc(resolutionLabels[status]||status)}: ${count}</div>`).join('')+'<p>Some calls may be missing, including those in macros or conditional code.</p>';
    function isBoundary(n){return !n.path}
    function isSupport(n){return ExplorerLanguages.isSupport(n)}
    function readableLabel(value,language){return ExplorerLanguages.readable(value,language)}
    function isBrowsable(n){return (showNoise||(!n.noise&&readableLabel(n.name,n.language)&&readableLabel(ownerName(n),n.language)))&&(showSupport||!isSupport(n))&&(showUncertain||!isBoundary(n))}
    function searchText(n){return [n.name,n.path,n.signature,n.module,...n.module_imports,...n.module_exports,...n.architecture_groups].filter(Boolean).join(' ').toLowerCase()}
    function ownerName(n){return ExplorerLanguages.owner(n.name,n.language)}
    function groupFor(n){if(perspective==='files')return n.path||'Boundaries';if(perspective==='modules')return n.module||'Not in a named module';if(perspective==='architecture')return n.primary_architecture_group||'Ungrouped';return ownerName(n)}
    function connectionCount(n){return (outgoing.get(n.id)?.length||0)+(incoming.get(n.id)?.length||0)}
    function nodeMeta(n){return [n.kind,n.signature,n.path].filter(Boolean).join(' · ')}
    function inspectNode(id){if(!nodes.has(id))return;inspected=id;selectedOccurrence=0;renderedCards=0;rightOpen=true;renderPaneState(false);renderSidebar();for(const group of document.querySelectorAll('.graph-node'))group.classList.toggle('inspected',group.dataset.id===id&&id!==selected)}
    function choose(id,push=true){if(!nodes.has(id))return;if(id===selected){inspectNode(id);return;}if(push&&selected)history.push(selected);selected=id;inspected=id;graphExtra=0;selectedOccurrence=0;renderedCards=0;viewport={x:0,y:0,scale:1};renderAll()}
    window.choose=choose;
    function renderTabs(){const p=document.getElementById('perspectives');p.innerHTML='';for(const [id,label] of perspectives){const b=document.createElement('button');b.textContent=label;b.id='perspective-'+id;b.setAttribute('role','tab');b.setAttribute('aria-selected',id===perspective);b.setAttribute('aria-controls','navigator');b.tabIndex=id===perspective?0:-1;b.onclick=()=>{perspective=id;activeGroup=null;groupLimit=12;renderTabs();renderNavigator();document.getElementById('perspective-'+id).focus()};b.onkeydown=e=>{const index=perspectives.findIndex(p=>p[0]===id),next=e.key==='ArrowRight'?(index+1)%perspectives.length:e.key==='ArrowLeft'?(index+perspectives.length-1)%perspectives.length:e.key==='Home'?0:e.key==='End'?perspectives.length-1:null;if(next!==null){e.preventDefault();document.getElementById('perspective-'+perspectives[next][0]).click()}};p.appendChild(b)}document.getElementById('navigator').setAttribute('aria-labelledby','perspective-'+perspective);const r=document.getElementById('rightTabs');r.innerHTML='';for(const [id,label] of detailTabs){const b=document.createElement('button');b.textContent=label;b.setAttribute('aria-pressed',id===detailTab);b.onclick=()=>{detailTab=id;renderTabs();renderSidebar()};r.appendChild(b)}}
    function navButton(n){return `<button class="nav-item${n.id===selected?' selected':''}" onclick="choose('${n.id}')"><strong>${esc(n.short_name||n.name)}</strong><small>${esc(nodeMeta(n))}</small></button>`}
    function renderNavigator(){const box=document.getElementById('navigator'),q=document.getElementById('search').value.trim().toLowerCase();const available=data.nodes.filter(isBrowsable);if(q){const matches=available.filter(n=>searchText(n).includes(q)).sort((a,b)=>connectionCount(b)-connectionCount(a)||a.name.localeCompare(b.name));box.innerHTML=`<div class="nav-heading"><strong>Search results</strong><span class="count">${matches.length}${matches.length>30?' · first 30 shown':''}</span></div>${matches.slice(0,30).map(navButton).join('')||'<div class="empty">No matching code found.</div>'}`;return}if(activeGroup){const matches=available.filter(n=>groupFor(n)===activeGroup).sort((a,b)=>connectionCount(b)-connectionCount(a)||a.name.localeCompare(b.name));box.innerHTML=`<div class="nav-heading"><button class="back" id="groupBack">← Groups</button><span class="count">${matches.length}${matches.length>30?' · first 30 shown':''}</span></div><div class="nav-heading"><strong>${esc(activeGroup)}</strong></div>${matches.slice(0,30).map(navButton).join('')}${matches.length>30?'<div class="notice">Use search to narrow this group.</div>':''}`;document.getElementById('groupBack').onclick=()=>{activeGroup=null;renderNavigator()};return}const groups=new Map();for(const n of available.filter(n=>n.path)){const g=groupFor(n);groups.set(g,(groups.get(g)||0)+1)}const ordered=[...groups].sort((a,b)=>b[1]-a[1]||a[0].localeCompare(b[0]));box.innerHTML=`<div class="nav-heading"><strong>${perspectives.find(p=>p[0]===perspective)[1]} groups</strong><span class="count">${ordered.length}</span></div>${ordered.slice(0,groupLimit).map(([g,count])=>`<button class="nav-item group-item" data-group="${esc(g)}"><strong>${esc(g)}</strong><small>${count} symbol${count===1?'':'s'}</small></button>`).join('')||'<div class="empty">No groups in this view.</div>'}${ordered.length>groupLimit?`<button class="show-more" id="moreGroups">Show ${ordered.length-groupLimit} more groups…</button>`:''}`;for(const b of box.querySelectorAll('.group-item'))b.onclick=()=>{activeGroup=b.dataset.group;renderNavigator()};const more=document.getElementById('moreGroups');if(more)more.onclick=()=>{groupLimit=ordered.length;renderNavigator()}}
    function relationAllowed(r){return ['exact','inferred'].includes(r.status)||showUncertain}
    function graphNodeAllowed(n){return n&&(isBrowsable(n)||n.id===selected)}
    function directChildren(n){if(!['class','struct','namespace'].includes(n.kind))return[];return data.nodes.filter(c=>c.path&&ExplorerLanguages.isDirectChild(n,c)&&graphNodeAllowed(c)).sort((a,b)=>connectionCount(b)-connectionCount(a)||a.name.localeCompare(b.name))}
    function graphNeighborhood(){const layer=new Map([[selected,0]]),edges=[],edgeKeys=new Set(),current=nodes.get(selected),height=Math.max(document.getElementById('canvasWrap').clientHeight,420),perLayer=Math.max(3,Math.min(7,Math.floor((height-90)/78)))+graphExtra;let omitted=0;const addEdge=(source,target,status,label)=>{const key=[source,target,status].join('|');if(!edgeKeys.has(key)){edgeKeys.add(key);edges.push({source,target,status,label})}};if(['class','struct','namespace'].includes(current.kind)){const children=directChildren(current),shown=children.slice(0,perLayer);for(const child of shown){layer.set(child.id,1);addEdge(current.id,child.id,'contains','contains')}omitted=children.length-shown.length;return{layer,edges,omitted,perLayer}}let frontier=[selected];for(let step=0;step<graphDepth&&frontier.length;step++){const candidates=new Map(),possible=[];const consider=(r,id,value)=>{const n=nodes.get(id);if(!relationAllowed(r)||!graphNodeAllowed(n))return;possible.push(r);if(layer.has(id))return;const prior=candidates.get(id),entry={id,layer:value,status:r.status};if(!prior||entry.status==='exact'&&prior.status!=='exact')candidates.set(id,entry)};for(const id of frontier){const value=layer.get(id)||0;if(value>=0)for(const r of outgoing.get(id)||[])consider(r,r.target,value+1);if(value<=0)for(const r of incoming.get(id)||[])consider(r,r.source,value-1)}const byLayer=new Map();for(const entry of candidates.values()){if(!byLayer.has(entry.layer))byLayer.set(entry.layer,[]);byLayer.get(entry.layer).push(entry)}const next=[];for(const [value,entries] of [...byLayer].sort((a,b)=>a[0]-b[0])){entries.sort((a,b)=>(a.status==='exact'?0:1)-(b.status==='exact'?0:1)||connectionCount(nodes.get(b.id))-connectionCount(nodes.get(a.id))||nodes.get(a.id).name.localeCompare(nodes.get(b.id).name));const shown=entries.slice(0,perLayer);omitted+=entries.length-shown.length;for(const entry of shown){layer.set(entry.id,value);next.push(entry.id)}}for(const r of possible)if(layer.has(r.source)&&layer.has(r.target))addEdge(r.source,r.target,r.status,r.status);frontier=next}for(const r of data.relations)if(relationAllowed(r)&&layer.has(r.source)&&layer.has(r.target))addEdge(r.source,r.target,r.status,r.status);return{layer,edges,omitted,perLayer}}
    function short(value,max=29){const text=String(value);return text.length>max?text.slice(0,max-1)+'…':text}
    function applyViewport(){const g=document.getElementById('viewport');if(g)g.setAttribute('transform',`translate(${viewport.x} ${viewport.y}) scale(${viewport.scale})`)}
    function zoomViewport(nextScale,clientX,clientY){const svg=document.getElementById('graphCanvas'),rect=svg.getBoundingClientRect(),view=svg.viewBox.baseVal,scale=Math.max(.35,Math.min(2.5,nextScale));if(scale===viewport.scale||!rect.width||!rect.height)return;const unit=Math.min(view.width/rect.width,view.height/rect.height),anchorX=(Number.isFinite(clientX)?clientX-rect.left:rect.width/2)*unit,anchorY=(Number.isFinite(clientY)?clientY-rect.top:rect.height/2)*unit,ratio=scale/viewport.scale;viewport.x=anchorX-(anchorX-viewport.x)*ratio;viewport.y=anchorY-(anchorY-viewport.y)*ratio;viewport.scale=scale;applyViewport()}
    function showGraphNotice(wrap,message){clearTimeout(noticeTimer);wrap.querySelector('.truncated')?.remove();if(!message)return;wrap.insertAdjacentHTML('beforeend',`<div class="truncated" role="status">${esc(message)}</div>`);noticeTimer=setTimeout(()=>wrap.querySelector('.truncated')?.remove(),3500)}
    function sourceSites(owner,target){return (outgoing.get(owner)||[]).filter(r=>r.target===target).flatMap(r=>r.evidence).filter(e=>e.path===nodes.get(owner)?.path).sort((a,b)=>a.line-b.line||a.column-b.column)}
    function hasFlowCall(flow){return flow.kind==='call'||flow.children.some(hasFlowCall)}
    function callCue(owner,site){
      let result=[];
      function visit(flow,cues){
        const next=[...cues];
        if(flow.kind==='loop')next.push('loop');
        if(flow.kind==='condition')next.push('condition');
        if(flow.kind==='branch')next.push('branch');
        if(flow.kind==='unspecified'&&flow.children.filter(hasFlowCall).length>1)next.push('order unspecified');
        if(flow.kind==='unsupported')next.push('partial structure');
        if(flow.kind==='call'&&flow.line===site?.line&&flow.column===site?.column)result=[...new Set(next)];
        for(const child of flow.children)visit(child,next)
      }
      const flow=nodes.get(owner)?.call_flow;if(flow&&site)visit(flow,[]);return result
    }
    function groupedPositions(layer,edges,centerX,centerY){
      const positions=new Map(),groups=[],info=new Map(),values=[...new Set(layer.values())].sort((a,b)=>a-b);
      let nextY=centerY+130;
      for(const value of values){
        const ids=[...layer].filter(([,v])=>v===value).map(([id])=>id);
        if(value<=0){ids.forEach((id,index)=>{const base=layoutPosition(value,index,ids.length,centerX,centerY,true),offset=nodeOffsets.get(id)||{x:0,y:0};positions.set(id,{x:base.x+offset.x,y:base.y+offset.y})});continue}
        const buckets=new Map();
        for(const id of ids){
          const owners=[...new Set(edges.filter(e=>e.target===id&&layer.get(e.source)===value-1).map(e=>e.source))].sort();
          const key=owners.join('|');if(!buckets.has(key))buckets.set(key,{owners,ids:[]});buckets.get(key).ids.push(id)
        }
        const tier=[...buckets.values()].sort((a,b)=>(positions.get(a.owners[0])?.x||0)-(positions.get(b.owners[0])?.x||0)||(positions.get(a.owners[0])?.y||0)-(positions.get(b.owners[0])?.y||0));
        let bottom=nextY;
        tier.forEach((group,index)=>{
          const owner=group.owners.length===1?group.owners[0]:null;
          group.ids.sort((a,b)=>{const x=sourceSites(owner,a)[0],y=sourceSites(owner,b)[0];return (x?.line??Infinity)-(y?.line??Infinity)||(x?.column??Infinity)-(y?.column??Infinity)||nodes.get(a).name.localeCompare(nodes.get(b).name)});
          const x=centerX+(index-(tier.length-1)/2)*(nodeW+110);
          group.ids.forEach((id,row)=>{
            const offset=nodeOffsets.get(id)||{x:0,y:0};positions.set(id,{x:x+offset.x,y:nextY+50+nodeH/2+row*(nodeH+28)+offset.y});
            const sites=sourceSites(owner,id),first=sites[0],cues=callCue(owner,first);
            info.set(id,{owners:group.owners,text:owner?[first?'L'+first.line:'Location unavailable',sites.length>1?sites.length+' call sites':'',...cues].filter(Boolean).join(' · '):'Shared by '+group.owners.length+' callers'})
          });
          group.label=owner?'Calls from '+(nodes.get(owner).short_name||nodes.get(owner).name):'Shared callees';
          group.caption=group.ids.length+(group.ids.length===1?' call · ':' calls · ')+(owner?'Source order':'no shared order');
          group.title=owner?'Ordered by first written call site, not runtime execution. Repeated calls share their function node.':'Shared function identity; incoming arrows retain the separate callers: '+group.owners.map(id=>nodes.get(id).name).join(', ');
          groups.push(group);bottom=Math.max(bottom,nextY+60+group.ids.length*(nodeH+28))
        });nextY=bottom+160
      }
      return {positions,groups,info}
    }
    function groupBounds(group){const points=group.ids.map(id=>graphPositions.get(id));return {x:Math.min(...points.map(p=>p.x))-nodeW/2-20,y:Math.min(...points.map(p=>p.y))-nodeH/2-48,width:Math.max(...points.map(p=>p.x))-Math.min(...points.map(p=>p.x))+nodeW+40,height:Math.max(...points.map(p=>p.y))-Math.min(...points.map(p=>p.y))+nodeH+68}}
    // Orthogonal group connectors avoid group interiors. Existing routes carry a
    // crossing/overlap cost so unrelated callers do not silently share a trunk.
    function orthogonalRoute(start,end,obstacles,used=[]){
      const xs=[...new Set([start.x,end.x,...obstacles.flatMap(b=>[b.x-48,b.x-32,b.x-16,b.x+b.width+16,b.x+b.width+32,b.x+b.width+48])])].sort((a,b)=>a-b);
      const ys=[...new Set([start.y,end.y,...obstacles.flatMap(b=>[b.y-32,b.y-16,b.y+b.height+16,b.y+b.height+32])])].sort((a,b)=>a-b);
      const inside=p=>obstacles.some(b=>p.x>b.x&&p.x<b.x+b.width&&p.y>b.y&&p.y<b.y+b.height);
      const clear=(a,b)=>!obstacles.some(r=>a.x===b.x?a.x>r.x&&a.x<r.x+r.width&&Math.max(a.y,b.y)>r.y&&Math.min(a.y,b.y)<r.y+r.height:a.y>r.y&&a.y<r.y+r.height&&Math.max(a.x,b.x)>r.x&&Math.min(a.x,b.x)<r.x+r.width);
      const penalty=(a,b)=>used.reduce((cost,[c,d])=>{
        if(a.x===b.x&&c.x===d.x&&Math.abs(a.x-c.x)<14)return cost+Math.max(0,Math.min(Math.max(a.y,b.y),Math.max(c.y,d.y))-Math.max(Math.min(a.y,b.y),Math.min(c.y,d.y)))*20;
        if(a.y===b.y&&c.y===d.y&&Math.abs(a.y-c.y)<14)return cost+Math.max(0,Math.min(Math.max(a.x,b.x),Math.max(c.x,d.x))-Math.max(Math.min(a.x,b.x),Math.min(c.x,d.x)))*20;
        const v=a.x===b.x?[a,b]:[c,d],h=a.x===b.x?[c,d]:[a,b];
        return cost+(v[0].x>Math.min(h[0].x,h[1].x)&&v[0].x<Math.max(h[0].x,h[1].x)&&h[0].y>Math.min(v[0].y,v[1].y)&&h[0].y<Math.max(v[0].y,v[1].y)?300:0)
      },0);
      const key=(x,y,dir)=>`${x},${y},${dir}`,queue=[],best=new Map(),previous=new Map();
      function push(item){queue.push(item);let i=queue.length-1;while(i){const parent=(i-1)>>1;if(queue[parent].score<=item.score)break;queue[i]=queue[parent];i=parent}queue[i]=item}
      function pop(){const first=queue[0],last=queue.pop();if(queue.length){let i=0;while(i*2+1<queue.length){let child=i*2+1;if(child+1<queue.length&&queue[child+1].score<queue[child].score)child++;if(queue[child].score>=last.score)break;queue[i]=queue[child];i=child}queue[i]=last}return first}
      const initial={x:xs.indexOf(start.x),y:ys.indexOf(start.y),dir:0,cost:0};initial.key=key(initial.x,initial.y,0);initial.score=0;push(initial);best.set(initial.key,0);
      while(queue.length){
        const current=pop();if(best.get(current.key)!==current.cost)continue;
        const a={x:xs[current.x],y:ys[current.y]};
        if(a.x===end.x&&a.y===end.y){const points=[a];let k=current.key;while(previous.has(k)){const step=previous.get(k);points.push(step.point);k=step.key}return points.reverse()}
        for(const [dx,dy,dir] of [[1,0,1],[-1,0,1],[0,1,2],[0,-1,2]]){
          const x=current.x+dx,y=current.y+dy;if(x<0||y<0||x>=xs.length||y>=ys.length)continue;
          const b={x:xs[x],y:ys[y]};if(inside(b)||!clear(a,b))continue;
          const cost=current.cost+Math.abs(a.x-b.x)+Math.abs(a.y-b.y)+(current.dir&&current.dir!==dir?28:0)+penalty(a,b),k=key(x,y,dir);
          if(cost>=(best.get(k)??Infinity))continue;best.set(k,cost);previous.set(k,{key:current.key,point:a});push({x,y,dir,cost,key:k,score:cost+Math.abs(b.x-end.x)+Math.abs(b.y-end.y)})
        }
      }
      return [start,{x:start.x,y:end.y},end] // Free dragging can place a port inside another group.
    }
    function rebuildGroupRoutes(){
      groupRoutes=[];if(!topDown||!groupedSequence)return;
      const obstacles=graphGroups.map(groupBounds);
      for(const [id,p] of graphPositions)if(!graphGroups.some(g=>g.ids.includes(id)))obstacles.push({x:p.x-nodeW/2,y:p.y-nodeH/2,width:nodeW,height:nodeH});
      const used=[];
      for(const [index,group] of graphGroups.entries())for(const [ownerIndex,owner] of group.owners.entries()){
        const a=graphPositions.get(owner);if(!a)continue;
        const parent=graphGroups.find(g=>g.ids.includes(owner)),bounds=groupBounds(group),parentBounds=parent&&groupBounds(parent);
        const side=bounds.x+bounds.width/2<=a.x?-1:1;
        const siblings=parent?graphGroups.flatMap(g=>g.owners.filter(id=>parent.ids.includes(id)&&((groupBounds(g).x+groupBounds(g).width/2<=graphPositions.get(id).x?-1:1)===side))).sort((a,b)=>graphPositions.get(a).y-graphPositions.get(b).y):[];
        const clearance=16+16*Math.max(0,siblings.length-1-siblings.indexOf(owner));
        const exitY=a.y+(parentBounds&&side<0?nodeH/2-12:0);
        const start={x:parentBounds?(side<0?parentBounds.x-clearance:parentBounds.x+parentBounds.width+clearance):a.x,y:parentBounds?exitY:a.y+nodeH/2+16};
        const port={x:a.x+(parentBounds?side*nodeW/2:0),y:parentBounds?exitY:a.y+nodeH/2};
        const end={x:bounds.x+bounds.width/2+(ownerIndex-(group.owners.length-1)/2)*24,y:bounds.y-16};
        const points=[port,...orthogonalRoute(start,end,obstacles,used),{x:end.x,y:bounds.y}];
        for(let i=1;i<points.length;i++)used.push([points[i-1],points[i]]);
        const relations=graphEdges.filter(e=>e.source===owner&&group.ids.includes(e.target)),statuses=[...new Set(relations.map(e=>e.status))];
        groupRoutes.push({owner,index,points,status:statuses.length===1?statuses[0]:'mixed',count:relations.length})
      }
    }
    function groupHtml(){return graphGroups.map((group,index)=>{const b=groupBounds(group);return `<g class="call-group" data-group-index="${index}"><rect x="${b.x}" y="${b.y}" width="${b.width}" height="${b.height}"></rect><text x="${b.x+12}" y="${b.y+19}">${esc(short(group.label,36))}</text><text class="group-caption" x="${b.x+12}" y="${b.y+35}">${esc(group.caption)}</text><title>${esc(group.title)}</title></g>`}).join('')+groupRoutes.map(route=>`<path class="group-entry edge ${route.status==='inferred'?'inferred':route.status==='exact'?'':'contains'}" data-owner="${route.owner}" d="${route.points.map((p,i)=>`${i?'L':'M'} ${p.x} ${p.y}`).join(' ')}" marker-end="url(#arrow)"><title>${esc(nodes.get(route.owner).name)} → ${esc(graphGroups[route.index].label)} · ${route.count} ${route.count===1?'call':'calls'}</title></path>`).join('')}
    function layoutPosition(value,index,count,centerX,centerY,vertical){
      const across=index-(count-1)/2;
      return vertical?{x:centerX+across*250,y:centerY+value*190}:{x:centerX+value*330,y:centerY+across*84}
    }
    function recenterGraph(){
      viewport={x:0,y:0,scale:1};
      if(topDown&&graphPositions.size){
        const view=document.getElementById('graphCanvas').viewBox.baseVal,points=[...graphPositions.values()];
        const left=Math.min(...points.map(p=>p.x))-nodeW/2-30,right=Math.max(...points.map(p=>p.x))+nodeW/2+30,top=Math.min(...points.map(p=>p.y))-nodeH/2-55,bottom=Math.max(...points.map(p=>p.y))+nodeH/2+40;
        const scale=Math.max(.35,Math.min(1,view.width/(right-left),view.height/(bottom-top)));
        viewport={scale,x:view.width/2-(left+right)/2*scale,y:view.height/2-(top+bottom)/2*scale}
      }
      applyViewport()
    }
    function setOrientation(vertical){
      topDown=vertical;document.getElementById('sequenceToggle').disabled=!vertical;nodeOffsets=orientationOffsets[vertical?(groupedSequence?'grouped':'vertical'):'horizontal'];
      viewport={x:0,y:0,scale:1};renderGraph(false);recenterGraph()
    }
    function renderGraph(refreshSidebar=true){
      const grouped=topDown&&groupedSequence;
      nodeH=(showFileNames?76:58)+(grouped?18:0);
      document.querySelector('.graph-hint').textContent=grouped?'Source order · click for details · double-click to explore · drag to arrange':'Click for details · double-click to explore · drag to arrange';
      const svg=document.getElementById('graphCanvas'),wrap=document.getElementById('canvasWrap'),current=nodes.get(selected);
      if(!current){svg.innerHTML='';return}
      const container=['class','struct','namespace'].includes(current.kind);
      document.getElementById('graphTitle').textContent=displayIdentity(current);document.getElementById('graphTitle').title=displayIdentity(current);
      const width=Math.max(wrap.clientWidth,520),height=Math.max(wrap.clientHeight,420),{layer,edges,omitted,perLayer}=graphNeighborhood(),layerValues=[...new Set(layer.values())].sort((a,b)=>a-b),positions=new Map(),columnGap=330;
      const hasLeft=layerValues.some(value=>value<0),hasRight=layerValues.some(value=>value>0),centerX=width/2+(topDown?0:hasLeft&&!hasRight?columnGap/2:hasRight&&!hasLeft?-columnGap/2:0),centerY=height/2+18+(topDown?(hasLeft&&!hasRight?95:hasRight&&!hasLeft?-95:0):0);
      for(const value of layerValues){
        const ids=[...layer].filter(([,v])=>v===value).map(([id])=>id).sort((a,b)=>connectionCount(nodes.get(b))-connectionCount(nodes.get(a))||nodes.get(a).name.localeCompare(nodes.get(b).name));
        ids.forEach((id,index)=>{const offset=nodeOffsets.get(id)||{x:0,y:0},base=layoutPosition(value,index,ids.length,centerX,centerY,topDown);positions.set(id,{x:base.x+offset.x,y:base.y+offset.y})})
      }
      graphGroups=[];groupedNodeInfo=new Map();
      if(grouped){const layout=groupedPositions(layer,edges,centerX,centerY);positions.clear();for(const [id,p] of layout.positions)positions.set(id,p);graphGroups=layout.groups;groupedNodeInfo=layout.info}
      graphPositions=positions;graphEdges=edges;rebuildGroupRoutes();
      const marker=`<defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill="context-stroke"></path></marker></defs>`;
      const edgeHtml=edges.map((edge,index)=>{const cls=edge.status==='exact'?'':edge.status==='inferred'?' inferred':edge.status==='contains'?' contains':' uncertain';return `<path class="edge${cls}" data-edge="${index}" d="${edgePath(edge)}" marker-end="url(#arrow)"><title>${esc(statusLabels[edge.status]||edge.label)}</title></path>`}).join('');
      const selectedName=short(current.short_name||current.name,18),nodeHtml=[...positions].map(([id,position])=>{const node=nodes.get(id),value=layer.get(id),relation=edges.find(edge=>value<0?edge.source===id:edge.target===id),certainty=relation?.status==='exact'?'exact match':relation?.status==='inferred'?'likely match':'',meta=id===selected?`${plainKind(node.kind)} · selected`:relation?.status==='contains'?`${plainKind(node.kind)} in ${selectedName}`:value===-1?`calls ${selectedName}${certainty?' · '+certainty:''}`:value===1?`called here${certainty?' · '+certainty:''}`:`${value<0?'upstream caller':'downstream call'}${certainty?' · '+certainty:''}`,classes=['graph-node',id===selected?'selected':'',isBoundary(node)?'boundary':'',node.noise?'noise':''].filter(Boolean).join(' ');return `<g class="${classes}" data-id="${id}" transform="translate(${position.x-nodeW/2} ${position.y-nodeH/2})" tabindex="0" role="button"><rect width="${nodeW}" height="${nodeH}" rx="9"></rect><text class="node-title" x="11" y="23">${esc(short(node.short_name||node.name,30))}</text><text class="node-meta" x="11" y="42">${esc(meta)}</text>${grouped&&groupedNodeInfo.has(id)?`<text class="node-meta" x="11" y="60">${esc(short(groupedNodeInfo.get(id).text,34))}<title>${esc(groupedNodeInfo.get(id).text)}</title></text>`:''}${showFileNames?`<text class="node-meta node-file" x="11" y="${grouped?80:62}">${esc(short(node.path?node.path.split('/').pop():'No local file',36))}<title>${esc(node.path||'No local file')}</title></text>`:''}<title>${esc(displayIdentity(node))}${node.signature?' · '+esc(node.signature):''}</title></g>`}).join('');
      const label=(value,text)=>topDown?`<text class="layer-label" text-anchor="start" style="text-anchor:start" x="${Math.min(...[...positions].filter(([id])=>layer.get(id)===value).map(([,p])=>p.x))-nodeW/2}" y="${centerY+value*190-nodeH/2-18}">${text}</text>`:`<text class="layer-label" x="${centerX+value*columnGap}" y="24">${text}</text>`;
      const labels=grouped?'':[layerValues.some(value=>value<0)&&label(-1,'CALLERS'),label(0,'FUNCTION'),layerValues.some(value=>value>0)&&label(1,container?'MEMBERS':'FUNCTION CALLS')].filter(Boolean).join('');
      svg.setAttribute('viewBox',`0 0 ${width} ${height}`);
      svg.innerHTML=marker+(topDown?'':labels)+`<g id="viewport"><g id="groupBoxes">${groupHtml()}</g>${edgeHtml}${nodeHtml}${topDown?labels:''}</g>`;
      for(const group of svg.querySelectorAll('.graph-node')){group.classList.toggle('inspected',group.dataset.id===inspected&&inspected!==selected);explorerInteractions.bind(group,{inspect:()=>inspectNode(group.dataset.id),explore:()=>choose(group.dataset.id),blocked:()=>suppressNodeClick})}
      showGraphNotice(wrap,layer.size===1?'No connected calls were found for this symbol.':'');
      if(omitted){clearTimeout(noticeTimer);wrap.querySelector('.truncated')?.remove();const notice=document.createElement('div');notice.className='truncated';notice.setAttribute('role','status');notice.style.pointerEvents='auto';notice.textContent=`${omitted} connections omitted · limit ${perLayer} per layer. `;if(graphExtra<35){const more=document.createElement('button');more.textContent='Show more';more.onclick=()=>{graphExtra+=7;renderGraph();recenterGraph()};notice.append(more)}else notice.append('Use the Calls tab for the full list.');wrap.append(notice)}
      applyViewport();if(refreshSidebar&&rightOpen&&detailTab!=='source')renderSidebar()
    }
    function routeEdge(from,to,width,height,vertical,self=false){
      // Route in flow/cross-flow coordinates, then rotate ports for top-down.
      const a=vertical?{x:from.y,y:from.x}:from,b=vertical?{x:to.y,y:to.x}:to;
      const w=vertical?height:width,h=vertical?width:height,point=(x,y)=>vertical?`${y} ${x}`:`${x} ${y}`;
      if(self)return `M ${point(a.x+w/2,a.y+10)} C ${point(a.x+w/2+65,a.y+70)}, ${point(a.x+w/2+65,a.y-70)}, ${point(a.x+w/2,a.y-10)}`;
      const dx=b.x-a.x,dy=b.y-a.y;
      if(Math.abs(dx)<24){const start=a.x+w/2,end=b.x+w/2,lane=Math.max(start,end)+100;return `M ${point(start,a.y)} C ${point(lane,a.y)}, ${point(lane,b.y)}, ${point(end,b.y)}`}
      if(Math.abs(dx)<w+24){const direction=dy>=0?1:-1,start=a.y+direction*h/2,end=b.y-direction*h/2,bend=Math.max(30,Math.abs(end-start)/2);return `M ${point(a.x,start)} C ${point(a.x,start+direction*bend)}, ${point(b.x,end-direction*bend)}, ${point(b.x,end)}`}
      const direction=dx>=0?1:-1,start=a.x+direction*w/2,end=b.x-direction*w/2,bend=Math.max(30,Math.abs(end-start)/2);
      return `M ${point(start,a.y)} C ${point(start+direction*bend,a.y)}, ${point(end-direction*bend,b.y)}, ${point(end,b.y)}`
    }
    function edgePath(edge){
      const a=graphPositions.get(edge.source),b=graphPositions.get(edge.target);
      if(a&&b&&topDown&&groupedSequence&&b.y>a.y+nodeH){
        const group=graphGroups.find(g=>g.ids.includes(edge.target)&&g.owners.includes(edge.source));
        if(group){const bounds=groupBounds(group),lane=bounds.x+7+group.owners.indexOf(edge.source)*4;return `M ${lane} ${b.y} L ${b.x-nodeW/2} ${b.y}`}
      }
      return a&&b?routeEdge(a,b,nodeW,nodeH,topDown,edge.source===edge.target):''
    }
    function updateDraggedNode(id){
      const position=graphPositions.get(id);
      for(const group of svg.querySelectorAll('.graph-node'))if(group.dataset.id===id)group.setAttribute('transform',`translate(${position.x-nodeW/2} ${position.y-nodeH/2})`);
      if(topDown&&groupedSequence){rebuildGroupRoutes();document.getElementById('groupBoxes').innerHTML=groupHtml()}
      for(const path of svg.querySelectorAll('.edge[data-edge]')){const edge=graphEdges[Number(path.dataset.edge)];if(topDown&&groupedSequence||edge.source===id||edge.target===id)path.setAttribute('d',edgePath(edge))}
    }
    const {sourceHtml}=createCallSource({outgoing,nodes,statusLabels,esc,languages:ExplorerLanguages});
    window.showOccurrence=index=>{selectedOccurrence=index;renderSidebar()};
    function plainKind(value){return ({method:'Method',function:'Function',class:'Class',struct:'Struct',namespace:'Namespace',lambda:'Lambda'}[value]||value||'Symbol')}
    function plainLink(value){return ({linked:'Declaration and definition linked','declaration-only':'Declaration only','definition-only':'Definition only',ambiguous:'Several possible definitions',unavailable:'Location unavailable'}[value]||value)}
    function plainForm(value){return ({free:'Function call',qualified:'Qualified function call',member:'Method call','pointer-member':'Pointer method call','static-member':'Static method call',constructor:'Constructor call',functor:'Callable object'}[value]||'Call')}
    function humanReason(value){if(!value)return '';if(value.includes('virtual dispatch'))return 'A virtual call may run a derived override.';if(value.includes('best syntactic candidate'))return '';if(value.includes('multiple equally plausible'))return 'Several candidates fit this call.';if(value.includes('parser recovery')||value.includes('conditional preprocessing'))return 'Macros or incomplete syntax prevent a reliable match.';return value}
    function sidebarRelations(n,direction){
      const items=(direction==='out'?outgoing.get(n.id):incoming.get(n.id))||[],other=r=>direction==='out'?r.target:r.source;
      const allowed=items.filter(r=>relationAllowed(r)&&graphNodeAllowed(nodes.get(other(r))));
      const shown=r=>graphEdges.some(e=>e.source===r.source&&e.target===r.target&&e.status===r.status);
      const inGraph=allowed.filter(shown).sort((a,b)=>(graphPositions.get(other(a))?.[topDown?'x':'y']||0)-(graphPositions.get(other(b))?.[topDown?'x':'y']||0)||nodes.get(other(a)).name.localeCompare(nodes.get(other(b)).name));
      const remaining=allowed.filter(r=>!shown(r)).sort((a,b)=>nodes.get(other(a)).name.localeCompare(nodes.get(other(b)).name));
      return {inGraph,remaining}
    }
    function relationSections(groups,render,empty){
      const shown=groups.inGraph.map(render).join(''),more=groups.remaining.length;
      return (shown||`<div class="empty">${empty}</div>`)+(more?`<details class="other-calls"><summary>${more} more · not shown in graph</summary>${groups.remaining.map(render).join('')}</details>`:'')
    }
    function renderDetails(n){const out=sidebarRelations(n,'out'),inc=sidebarRelations(n,'in'),tags=[plainKind(n.kind),plainLink(n.link_status),n.module&&`Module ${n.module}`,...n.architecture_groups].filter(Boolean);const module=n.module?`<div class="section"><h3>Module ${esc(n.module)}</h3><div class="signature"><strong>Uses</strong><br>${n.module_imports.map(esc).join('<br>')||'No module imports found'}<br><br><strong>Makes available</strong><br>${n.module_exports.map(esc).join('<br>')||'No exports found'}</div></div>`:'';const links=(groups,direction,empty)=>relationSections(groups,r=>{const x=nodes.get(direction==='out'?r.target:r.source);return `<button class="relation-link status-${esc(r.status)}" title="${esc(statusLabels[r.status]||r.status)}" aria-label="${esc(x.name)}: ${esc(statusLabels[r.status]||r.status)}" onclick="choose('${x.id}')">${esc(x.name)}</button>`},empty);return `<div class="section symbol-header"><h2>${esc(n.name)}</h2><div class="symbol-meta">${tags.map(esc).join(' · ')}</div>${n.signature?`<div class="signature"><code>${esc(n.signature)}</code></div>`:''}<div class="location">${esc(n.path||'No local file')}</div></div><div class="section"><h3>Statistics</h3><dl class="stats"><dt>Exact callers</dt><dd>${n.fan_in}</dd><dt>Exact calls made</dt><dd>${n.fan_out}</dd><dt>Source lines</dt><dd>${n.source?.lines?.length??'—'}</dd></dl><div class="notice">Call counts exclude likely matches.</div></div>${module}<div class="section"><h3>Called by</h3>${links(inc,'in','No callers in this graph.')}</div><div class="section"><h3>Calls from here</h3>${links(out,'out','No calls from this symbol in the graph.')}</div>`}
    function renderSource(n){const occurrences=n.occurrences||[],source=occurrences.length?occurrences[selectedOccurrence]?.source:n.source,occurrenceLabel=o=>o.kind==='definition'?'Definition':'Declaration';return `<div class="section"><h2>${esc(n.short_name||n.name)}</h2><div class="location">${esc(n.path||'No local source')}</div></div><div class="section"><h3>Where this symbol appears</h3><div class="source-tools">${occurrences.map((o,i)=>`<button aria-pressed="${i===selectedOccurrence}" onclick="showOccurrence(${i})">${occurrenceLabel(o)} ${esc(o.label)}</button>`).join('')||'<span class="empty">No source was embedded for this symbol.</span>'}</div>${sourceHtml(source,[],n.id,occurrences.length?occurrences[selectedOccurrence]?.label.replace(/:\d+:\d+$/,''):n.path)}</div>`}
    window.toggleProjection=(button,targetId,depth,ancestryText,relationIndex)=>{const host=button.closest('.evidence'),old=host.querySelector(':scope > .projection');if(old){old.remove();button.textContent='Code';renderedCards=Math.max(0,renderedCards-1);return}const ancestry=ancestryText?ancestryText.split('|'):[];if(renderedCards>=100){host.insertAdjacentHTML('beforeend','<div class="projection notice">Too many code previews are open. Close one before opening another.</div>');return}if(depth>=data.max_expansion_depth){host.insertAdjacentHTML('beforeend',`<div class="projection notice">This preview stops after ${data.max_expansion_depth} nested calls.</div>`);return}if(ancestry.includes(targetId)){host.insertAdjacentHTML('beforeend','<div class="projection notice">This call leads back to a function already shown above.</div>');return}const target=nodes.get(targetId),relation=data.relations[relationIndex],evidence=relation?.evidence[0],caller=nodes.get(relation?.source);
      const callerSource=evidence&&((caller?.occurrences||[]).find(o=>o.label.startsWith(evidence.path+':')&&o.source.start_line<=evidence.line&&o.source.end_line>=evidence.line)?.source||(caller?.path===evidence.path?caller.source:null));
      const hasCallSite=callerSource&&evidence&&callerSource.start_line<=evidence.line&&callerSource.end_line>=evidence.line;
      const highlights=hasCallSite?[evidence.line]:[];
      let callSite='';
      if(evidence&&relation.source!==targetId){
        const first=hasCallSite?Math.max(callerSource.start_line,evidence.line-2):0,last=hasCallSite?Math.min(callerSource.end_line,evidence.line+2):0;
        const context=hasCallSite?{start_line:first,start_column:first===callerSource.start_line?callerSource.start_column:1,end_line:last,lines:callerSource.lines.slice(first-callerSource.start_line,last-callerSource.start_line+1)}:null;
        callSite=`<strong>Call site · ${esc(evidence.path)}:${evidence.line}</strong>${context?sourceHtml(context,highlights,caller.id,evidence.path):'<div class="notice">Caller source is not embedded for this location.</div>'}`
      }
      renderedCards++;button.textContent='Hide code';host.insertAdjacentHTML('beforeend',`<div class="projection">${callSite}<strong>Function code · ${esc(target.name)}</strong>${sourceHtml(target.source,relation?.source===targetId?highlights:[],targetId,target.path)}</div>`)};
    function callCard(r,target,n){const ev=r.evidence[0]||{},expand=target?.source&&['exact','inferred'].includes(r.status)?`<button onclick="toggleProjection(this,'${target.id}',0,'${n.id}',${data.relations.indexOf(r)})">Code</button>`:'',reason=humanReason(r.reason);return `<div class="evidence"><button class="relation-link status-${esc(r.status)}" title="${esc(statusLabels[r.status]||r.status)}" aria-label="${esc(target.name)}: ${esc(statusLabels[r.status]||r.status)}" onclick="choose('${target.id}')"><strong>${esc(target.name)}</strong></button><div class="call-expression">${esc(ev.expression||'Call expression unavailable')}</div><div class="location">${plainForm(ev.form)} · ${esc(ev.path||'Unknown file')}:${ev.line||'?'}:${ev.column||'?'}</div>${reason?`<div class="notice">${esc(reason)}</div>`:''}${r.alternatives.length?`<div class="notice"><strong>Other possible matches:</strong><br>${r.alternatives.map(a=>esc(a.name)+(a.signature?' · '+esc(a.signature):'')).join('<br>')}</div>`:''}${expand}</div>`}
    function renderCalls(n){const out=sidebarRelations(n,'out'),inc=sidebarRelations(n,'in'),cards=(groups,direction,empty)=>relationSections(groups,r=>callCard(r,nodes.get(direction==='out'?r.target:r.source),n),empty);return `<div class="section"><h2>Calls</h2><div class="location">${esc(n.name)}</div>${n.link_status==='ambiguous'?'<div class="notice">Multiple definitions share this symbol. Calls below may come from different files.</div>':''}</div><div class="section"><h3>Calls made here · in graph</h3>${cards(out,'out','No outgoing calls in this graph.')}</div><div class="section"><h3>Called from · in graph</h3>${cards(inc,'in','No callers in this graph.')}</div>`}
    function renderSidebar(){const n=nodes.get(inspected);if(!n){document.getElementById('rightBody').innerHTML='<div class="empty">Select a graph node.</div>';return}document.getElementById('rightBody').innerHTML=detailTab==='source'?renderSource(n):detailTab==='calls'?renderCalls(n):renderDetails(n)}
    function renderDepth(){document.getElementById('depthSlider').value=graphDepth;document.getElementById('depthInput').value=graphDepth}
    function setGraphDepth(value){const parsed=Number.parseInt(value,10);graphDepth=Math.max(1,Math.min(10,Number.isFinite(parsed)?parsed:1));viewport={x:0,y:0,scale:1};renderDepth();renderGraph();if(topDown)recenterGraph()}
    function renderSearchState(){const search=document.getElementById('search');document.getElementById('clearSearch').hidden=!search.value}
    function setSearch(value){document.getElementById('search').value=value;renderSearchState();renderNavigator()}
    function renderPaneState(relayout=true){const layout=document.getElementById('layout'),left=document.getElementById('leftPaneToggle'),right=document.getElementById('rightPaneToggle');layout.classList.toggle('hide-left',!leftOpen);layout.classList.toggle('hide-right',!rightOpen);ExplorerRuntime.paneToggle(left,leftOpen,'navigator');ExplorerRuntime.paneToggle(right,rightOpen,'details');if(relayout)requestAnimationFrame(renderGraph)}
    function goHome(){setSearch('');selected=homeSelection;inspected=selected;perspective=data.nodes.some(n=>n.primary_architecture_group)?'architecture':'symbols';activeGroup=null;detailTab='details';graphDepth=1;groupLimit=12;history=[];selectedOccurrence=0;viewport={x:0,y:0,scale:1};leftOpen=true;rightOpen=false;renderAll()}
    function renderAll(){renderTabs();renderDepth();renderSearchState();renderNavigator();renderGraph();renderSidebar();renderPaneState();document.getElementById('backButton').hidden=!history.length}
    document.getElementById('homeButton').onclick=goHome;document.getElementById('search').addEventListener('input',e=>setSearch(e.target.value));document.getElementById('clearSearch').onclick=()=>{setSearch('');document.getElementById('search').focus()};ExplorerRuntime.bindDepth(document.getElementById('depthSlider'),document.getElementById('depthInput'),setGraphDepth);document.getElementById('uncertainToggle').onchange=e=>{showUncertain=e.target.checked;renderAll()};document.getElementById('supportToggle').onchange=e=>{showSupport=e.target.checked;renderAll()};document.getElementById('noiseToggle').onchange=e=>{showNoise=e.target.checked;renderAll()};document.getElementById('leftPaneToggle').onclick=()=>{leftOpen=!leftOpen;renderPaneState()};document.getElementById('rightPaneToggle').onclick=()=>{rightOpen=!rightOpen;renderPaneState()};document.getElementById('backButton').onclick=()=>{const id=history.pop();if(id){selected=id;inspected=id;graphExtra=0;selectedOccurrence=0;viewport={x:0,y:0,scale:1};renderAll()}};document.getElementById('zoomIn').onclick=()=>zoomViewport(viewport.scale*1.2);document.getElementById('zoomOut').onclick=()=>zoomViewport(viewport.scale/1.2);document.getElementById('resetView').onclick=()=>{renderGraph(false);recenterGraph()};document.getElementById('resetLayout').onclick=()=>{nodeOffsets.clear();viewport={x:0,y:0,scale:1};renderGraph(false);recenterGraph()};
    const paneWidths={left:260,right:360};
    function resizePane(side,width){
      const layout=document.getElementById('layout'),other=side==='left'?(rightOpen?paneWidths.right:0):(leftOpen?paneWidths.left:0),minimum=side==='left'?200:240,maximum=Math.max(minimum,layout.clientWidth-other-360);
      paneWidths[side]=Math.max(minimum,Math.min(maximum,width));
      layout.style.setProperty('--'+side+'-width',paneWidths[side]+'px');
      const handle=document.getElementById(side+'Resizer');
      handle.setAttribute('aria-valuemin',minimum);handle.setAttribute('aria-valuemax',maximum);handle.setAttribute('aria-valuenow',paneWidths[side]);
      // Resizing must not close code previews already open in the sidebar.
      renderGraph(false)
    }
    for(const side of ['left','right']) ExplorerRuntime.bindResizer(document.getElementById(side+'Resizer'), {
      read:()=>paneWidths[side], write:width=>resizePane(side,width), initial:side==='left'?260:360, sign:side==='left'?1:-1,
      cancelInspection:()=>explorerInteractions.cancelInspection()
    });
    document.getElementById('sequenceToggle').onchange=event=>{groupedSequence=event.target.checked;setOrientation(topDown)};
    document.getElementById('orientationToggle').onchange=event=>setOrientation(event.target.checked);
    document.getElementById('fileNamesToggle').onchange=event=>{showFileNames=event.target.checked;renderGraph(false)};
    function wheelZoomDelta(e){return Math.abs(e.deltaY)>=Math.abs(e.deltaX)?e.deltaY:e.deltaX}
    const svg=document.getElementById('graphCanvas');svg.addEventListener('wheel',e=>{const delta=wheelZoomDelta(e);if(!delta)return;e.preventDefault();zoomViewport(viewport.scale*(delta<0?1.1:.9),e.clientX,e.clientY)},{passive:false});svg.addEventListener('pointerdown',e=>{
      if(e.button!==0||drag)return;
      suppressNodeClick=false;
      const id=e.target.closest('.graph-node')?.dataset.id,position=graphPositions.get(id),offset=nodeOffsets.get(id)||{x:0,y:0};
      drag={pointerId:e.pointerId,id,x:e.clientX,y:e.clientY,ox:viewport.x,oy:viewport.y,position:position&&{...position},offset:{...offset},moved:false};
      drag.capture=e.target.closest('.graph-node')||svg;drag.capture.setPointerCapture(e.pointerId)
    });
    svg.addEventListener('pointermove',e=>{
      if(!drag||e.pointerId!==drag.pointerId)return;
      const dx=e.clientX-drag.x,dy=e.clientY-drag.y;
      if(!drag.moved&&Math.hypot(dx,dy)<4)return;
      drag.moved=true;
      const rect=svg.getBoundingClientRect(),view=svg.viewBox.baseVal,sx=Math.min(view.width/rect.width,view.height/rect.height),sy=sx;
      if(drag.id){const x=dx*sx/viewport.scale,y=dy*sy/viewport.scale;nodeOffsets.set(drag.id,{x:drag.offset.x+x,y:drag.offset.y+y});graphPositions.set(drag.id,{x:drag.position.x+x,y:drag.position.y+y});updateDraggedNode(drag.id)}
      else{viewport.x=drag.ox+dx*sx;viewport.y=drag.oy+dy*sy;applyViewport()}
    });
    function finishDrag(e){
      if(!drag||e.pointerId!==drag.pointerId)return;
      const completed=drag;drag=null;suppressNodeClick=completed.moved;
      if(completed.capture.hasPointerCapture(e.pointerId))completed.capture.releasePointerCapture(e.pointerId);
      // Native click/double-click stays on the captured node. Movement never activates it.
    }
    svg.addEventListener('pointerup',finishDrag);svg.addEventListener('pointercancel',finishDrag);svg.addEventListener('lostpointercapture',finishDrag);
    window.addEventListener('resize',renderGraph);
    renderAll();
