      // This file is embedded inside the viewer closure; no network requests.
      let suppressClick=false, editingGroup=null, fileLimit=100, graphLimit=150, graphOmitted=0;
      const workspace=document.querySelector('.workspace');
      const field=id=>document.getElementById(id);
      const localNodes=data.nodes.filter(dependencyTools.local);
      state.architecture.groups=[...new Set(localNodes.map(n=>(n.path||n.name).split('/')[0]).filter(p=>localNodes.some(n=>(n.path||n.name).startsWith(p+'/'))))].sort().map(name=>({name,prefixes:[name]}));
      function remember(){
        graphLimit=150;
        state.history.push({mode:state.mode,selected:state.selected,inspected:state.inspected,currentGroup:state.currentGroup,archFocus:state.archFocus,path:state.path,direction:state.direction,depth:state.depth,closure:state.closure});
        if(state.history.length>100)state.history.shift();
        field('back').hidden=false;
      }
      function refreshFeatureState(){
        field('back').hidden=!state.history.length;
        const limitNotice=field('graph-limit');limitNotice.hidden=!graphOmitted;
        limitNotice.textContent=graphOmitted?`${state.items.length} nodes shown · ${graphOmitted} omitted. Select a file or group to narrow the graph.`:'';
        if(graphOmitted&&graphLimit<900){const more=document.createElement('button');more.textContent='Show more';more.onclick=()=>{graphLimit+=150;render()};limitNotice.append(more)}
        field('depth').value=state.depth;field('depth').disabled=state.closure;field('depth-slider').value=state.depth;field('depth-slider').disabled=state.closure;
        field('direction').value=state.direction;field('closure').checked=state.closure;
        const focus=nodeById.get(state.selected);
        field('focus-title').textContent=focus?.name||({overview:'Dependencies',architecture:state.archFocus||'Architecture',path:'Dependency path',full:'Full graph'}[state.mode]||'Dependencies');field('focus-title').title=field('focus-title').textContent;
        field('trace-message').textContent=state.mode==='path'?(state.path?`${state.path.length-1} steps · exact project dependencies`:'No exact project path found.'):state.mode==='neighborhood'?`${focus?.short_name||'Connections'} · ${state.items.length} nodes${state.closure?' · all reachable':''}`:'Click for details · double-click to explore · drag to arrange.';
        document.querySelector('[data-view="neighborhood"]').disabled=!state.selected;
      }
      function refreshNavigator(){
        const query=state.query.trim().toLowerCase();
        const matches=localNodes.filter(n=>!query||`${n.name} ${n.path||''}`.toLowerCase().includes(query));
        field('file-list').innerHTML=`<p class="empty-list">${matches.length} ${matches.length===1?'file':'files'}${matches.length>fileLimit?` · first ${fileLimit} shown`:''}</p>`+matches.slice(0,fileLimit).map(n=>`<button class="node-link file-row" data-file="${escapeHtml(n.id)}" aria-label="${escapeHtml(n.name)}" title="${escapeHtml(n.path||n.name)}"><strong>${escapeHtml(n.short_name||n.name.split('/').pop())}</strong><small>${escapeHtml(n.path||n.name)}</small></button>`).join('')||'<p class="empty-list">No matches.</p>';
        if(matches.length>fileLimit){const more=document.createElement('button');more.textContent='Show more files';more.onclick=()=>{fileLimit+=100;refreshNavigator()};field('file-list').append(more)}
        field('file-list').querySelectorAll('[data-file]').forEach(b=>b.onclick=()=>selectOriginalNode(b.dataset.file,true));
      }
      function architectureGraph(){
        const members=dependencyTools.membership(data.nodes,state.architecture),visible=data.nodes.filter(visibleNode),groups=new Map(),items=[],mapping=new Map();
        if(state.archFocus!==null){
          const ids=new Set(visible.filter(n=>(members.get(n.id)||'Ungrouped')===state.archFocus).map(n=>n.id));
          return {items:visible.filter(n=>ids.has(n.id)).map(n=>({...n,members:[n.id]})),relations:rawVisibleRelations().filter(r=>ids.has(r.source)&&ids.has(r.target))};
        }
        for(const n of visible){const name=members.get(n.id)||'Ungrouped';if(!groups.has(name))groups.set(name,[]);groups.get(name).push(n.id)}
        for(const [name,ids] of groups){const id='architecture-'+encodeURIComponent(name);items.push({id,name,short_name:name,kind:'architecture',subtitle:`${ids.length} files or modules`,members:ids,archGroup:name});for(const member of ids)mapping.set(member,id)}
        const combined=new Map();
        for(const r of rawVisibleRelations()){const source=mapping.get(r.source),target=mapping.get(r.target);if(!source||!target||source===target)continue;const key=JSON.stringify([source,target,r.kind]);if(!combined.has(key))combined.set(key,{source,target,kind:r.kind,originals:[]});combined.get(key).originals.push(r)}
        return {items,relations:[...combined.values()]};
      }
      function showArchitectureDetails(item){
        const ids=new Set(item.members),cross=data.relations.filter(r=>ids.has(r.source)&&!ids.has(r.target));
        details.innerHTML=`<h2>${escapeHtml(item.name)}</h2><p>${item.members.length} files or modules</p><h3>Members</h3>${linkList(item.members.map(id=>nodeById.get(id)))}<h3>Dependencies outside this group</h3>${linkList(cross.map(r=>nodeById.get(r.target)))}`;bindNodeLinks();
      }
      function refreshArchitecture(){
        field('group-list').innerHTML=state.architecture.groups.map((g,i)=>`<div class="group-row"><strong>${escapeHtml(g.name)}</strong><br><span class="empty-list">${g.prefixes.map(escapeHtml).join(', ')}</span><br><button data-edit-group="${i}">Edit</button> <button data-remove-group="${i}">Remove</button></div>`).join('');
        field('group-list').querySelectorAll('[data-edit-group]').forEach(b=>b.onclick=()=>{const g=state.architecture.groups[Number(b.dataset.editGroup)];editingGroup=g.name;field('group-name').value=g.name;field('group-prefixes').value=g.prefixes.join('\n')});
        field('group-list').querySelectorAll('[data-remove-group]').forEach(b=>b.onclick=()=>{const name=state.architecture.groups[Number(b.dataset.removeGroup)].name;state.architecture.groups=state.architecture.groups.filter(g=>g.name!==name);state.architecture.rules=state.architecture.rules.filter(r=>r.from!==name&&r.to!==name);state.archFocus=null;refreshArchitecture();render()});
        const options=state.architecture.groups.map(g=>`<option value="${escapeHtml(g.name)}">${escapeHtml(g.name)}</option>`).join('');field('rule-from').innerHTML=options;field('rule-to').innerHTML=options;
        field('rule-list').innerHTML=state.architecture.rules.map((r,i)=>`<div class="group-row">${escapeHtml(r.from)} → ${escapeHtml(r.to)} <button data-remove-rule="${i}" aria-label="Remove rule ${i+1}">×</button></div>`).join('');
        field('rule-list').querySelectorAll('[data-remove-rule]').forEach(b=>b.onclick=()=>{state.architecture.rules.splice(Number(b.dataset.removeRule),1);refreshArchitecture();render(false)});
      }
      function checkRules(){
        const results=dependencyTools.violations(data.nodes,data.relations,state.architecture),exact=results.filter(r=>r.confirmed),possible=results.filter(r=>!r.confirmed);
        const rows=entries=>entries.map((r,i)=>`<button class="node-link" data-violation="${results.indexOf(r)}">${escapeHtml(nodeById.get(r.source).name)} → ${escapeHtml(nodeById.get(r.target).name)}</button>`).join('')||'<p class="empty-list">None.</p>';
        details.innerHTML=`<h2>Dependency rules</h2><p>Checked ${state.architecture.rules.length} rules against this report. Exact matches are violations; uncertain matches need review.</p><h3>${exact.length} violations</h3>${rows(exact)}<h3>${possible.length} uncertain matches</h3>${rows(possible)}`;
        details.querySelectorAll('[data-violation]').forEach(b=>b.onclick=()=>{const r=results[Number(b.dataset.violation)];showEdgeDetails(r,nodeById.get(r.source),nodeById.get(r.target))});
        workspace.classList.remove('hide-details');field('toggle-details').textContent='Hide details';field('toggle-details').setAttribute('aria-pressed','true');
      }
      function downloadJson(name,value){const url=URL.createObjectURL(new Blob([JSON.stringify(value,null,2)],{type:'application/json'})),a=document.createElement('a');a.href=url;a.download=name;document.body.appendChild(a);a.click();a.remove();setTimeout(()=>URL.revokeObjectURL(url),60000)}
      async function readJson(input){const file=input.files?.[0];if(!file)return null;if(file.size>25*1024*1024)throw Error('Choose a file smaller than 25 MB.');return JSON.parse(await file.text())}
      function comparison(){
        if(!state.baseline){field('compare-results').innerHTML='';field('clear-baseline').hidden=true;return}
        const result=dependencyTools.compare(data,state.baseline);field('clear-baseline').hidden=false;
        field('compare-message').textContent=`Baseline: ${state.baseline.label}. ${result.added.length} added; ${result.removed.length} removed. Match-status changes count as a removal and an addition. Comparison covers the reports as generated, not just the visible filters.`;
        const rows=(entries,kind)=>entries.map((r,i)=>`<div class="change-${kind}">${escapeHtml(r.source.name)} → ${escapeHtml(r.target.name)}<br><span class="empty-list">${escapeHtml(matchLabels[r.kind]||r.kind)}</span>${kind==='added'?`<button data-added="${i}">Show</button>`:''}</div>`).join('')||'<p class="empty-list">None.</p>';
        field('compare-results').innerHTML=`<h3>Added dependencies</h3>${rows(result.added,'added')}<h3>Removed dependencies</h3>${rows(result.removed,'removed')}`;
        field('compare-results').querySelectorAll('[data-added]').forEach(b=>b.onclick=()=>{const r=result.added[Number(b.dataset.added)];selectOriginalNode(r.source.id,true);const relation=data.relations.find(e=>e.source===r.source.id&&e.target===r.target.id&&e.kind===r.kind);showEdgeDetails(relation,r.source,r.target)});
      }
      function bindNodeDrag(element,item){
        let moving=null;
        element.addEventListener('pointerdown',event=>{if(event.button!==0)return;event.stopPropagation();const p=state.positions.get(item.id);moving={x:event.clientX,y:event.clientY,start:{...p},moved:false};element.setPointerCapture(event.pointerId)});
        element.addEventListener('pointermove',event=>{if(!moving)return;const dx=(event.clientX-moving.x)/state.transform.scale,dy=(event.clientY-moving.y)/state.transform.scale;if(Math.abs(dx)+Math.abs(dy)<3&&!moving.moved)return;moving.moved=true;const p={x:moving.start.x+dx,y:moving.start.y+dy};state.positions.set(item.id,p);element.setAttribute('transform',`translate(${p.x} ${p.y})`);edgeLayer.replaceChildren(...state.relations.map(renderEdge));updateSelectionStyles()});
        function end(){if(moving?.moved){const offsets=state.offsets[state.topDown?'vertical':'horizontal'],previous=offsets.get(item.id)||{x:0,y:0},p=state.positions.get(item.id);offsets.set(item.id,{x:previous.x+p.x-moving.start.x,y:previous.y+p.y-moving.start.y});suppressClick=true;setTimeout(()=>suppressClick=false,0)}moving=null}
        element.addEventListener('pointerup',end);element.addEventListener('pointercancel',end);
      }
      function resizePanel(id,property,defaultWidth,sign){
        const handle=field(id);let resizing=null;
        function setWidth(value){const other=property==='--nav-width'?document.querySelector('.sidebar'):document.querySelector('.navigator');const max=Math.max(180,Math.min(600,workspace.clientWidth-other.getBoundingClientRect().width-360));const width=Math.max(180,Math.min(max,value));workspace.style.setProperty(property,width+'px');handle.setAttribute('aria-valuenow',width);handle.setAttribute('aria-valuemin','180');handle.setAttribute('aria-valuemax',max);fitGraph()}
        handle.addEventListener('pointerdown',e=>{if(e.button!==0)return;resizing={x:e.clientX,width:handle.parentElement.getBoundingClientRect().width};handle.setPointerCapture(e.pointerId);e.preventDefault()});
        handle.addEventListener('pointermove',e=>{if(resizing)setWidth(resizing.width+(e.clientX-resizing.x)*sign)});
        handle.addEventListener('pointerup',()=>resizing=null);handle.addEventListener('pointercancel',()=>resizing=null);
        handle.addEventListener('keydown',e=>{if(['ArrowLeft','ArrowRight','Home'].includes(e.key)){e.preventDefault();setWidth(e.key==='Home'?defaultWidth:handle.parentElement.getBoundingClientRect().width+(e.key==='ArrowRight'?20:-20)*sign)}});
      }
      field('back').onclick=()=>{const previous=state.history.pop();if(!previous)return;Object.assign(state,previous);state.selectedEdge=null;render();if(nodeById.has(state.inspected||state.selected))showNodeDetails(nodeById.get(state.inspected||state.selected));else showPlaceholder()};
      function updateTrace(){remember();state.direction=field('direction').value;state.depth=Math.max(0,Math.min(20,Number.parseInt(field('depth').value,10)||0));state.closure=field('closure').checked;if(state.selected)state.mode='neighborhood';render()}
      field('depth-slider').oninput=e=>{field('depth').value=e.target.value;updateTrace()};field('depth').onchange=updateTrace;field('depth').onkeydown=e=>{if(e.key==='Enter'){e.preventDefault();updateTrace()}};field('direction').onchange=updateTrace;field('closure').onchange=updateTrace;
      field('top-down').onchange=e=>{state.topDown=e.target.checked;render()};
      field('reset-layout').onclick=()=>{state.offsets[state.topDown?'vertical':'horizontal'].clear();render()};
      field('path-form').onsubmit=e=>{e.preventDefault();const from=localNodes.find(n=>n.name===field('path-from').value),to=localNodes.find(n=>n.name===field('path-to').value);if(!from||!to){field('trace-message').textContent='Choose two exact file or module names from the suggestions.';return}remember();state.path=dependencyTools.path(data.nodes,data.relations,from.id,to.id);state.mode='path';state.selected=null;state.inspected=null;render();details.innerHTML=`<h2>Dependency path</h2><p>Exact project dependencies only. Display filters do not change this query.</p>${state.path?linkList(state.path.map(id=>nodeById.get(id))):'<p>No path found.</p>'}`;bindNodeLinks()};
      for(const panel of ['nav','details'])field('toggle-'+panel).onclick=()=>{const hidden=workspace.classList.toggle('hide-'+panel);field('toggle-'+panel).textContent=(hidden?'Show ':'Hide ')+(panel==='nav'?'navigator':'details');field('toggle-'+panel).setAttribute('aria-pressed',String(!hidden));fitGraph()};
      resizePanel('resize-nav','--nav-width',250,1);resizePanel('resize-details','--details-width',330,-1);
      document.querySelectorAll('[data-panel]').forEach(b=>b.onclick=()=>{state.navPanel=b.dataset.panel;document.querySelectorAll('[data-panel]').forEach(x=>x.setAttribute('aria-pressed',String(x===b)));for(const name of ['files','architecture','compare'])field('nav-'+name).hidden=name!==state.navPanel});
      search.addEventListener('input',()=>{fileLimit=100;refreshNavigator()});
      field('architecture-view').onclick=()=>{remember();state.mode='architecture';state.archFocus=null;state.selected=null;state.inspected=null;render();details.innerHTML='<h2>Architecture</h2><p>Select a group to see its files. Arrows show dependencies between groups.</p>'};
      field('group-cancel').onclick=()=>{editingGroup=null;field('group-form').reset()};
      field('group-form').onsubmit=e=>{e.preventDefault();try{const name=field('group-name').value.trim(),groups=state.architecture.groups.filter(g=>g.name!==editingGroup);groups.push({name,prefixes:field('group-prefixes').value.split('\n').filter(p=>p.trim())});const rules=state.architecture.rules.map(r=>({from:r.from===editingGroup?name:r.from,to:r.to===editingGroup?name:r.to}));state.architecture=dependencyTools.validateArchitecture({...state.architecture,groups,rules});editingGroup=null;state.archFocus=null;field('group-form').reset();field('architecture-message').textContent='Group saved for this page. Save configuration to keep it.';refreshArchitecture();render()}catch(error){field('architecture-message').textContent=error.message}};
      field('rule-form').onsubmit=e=>{e.preventDefault();try{const rule={from:field('rule-from').value,to:field('rule-to').value};if(state.architecture.rules.some(r=>r.from===rule.from&&r.to===rule.to))throw Error('This rule already exists.');state.architecture=dependencyTools.validateArchitecture({...state.architecture,rules:[...state.architecture.rules,rule]});refreshArchitecture();render(false);checkRules()}catch(error){field('architecture-message').textContent=error.message}};
      field('check-rules').onclick=checkRules;field('save-architecture').onclick=()=>downloadJson('dependency-architecture.json',state.architecture);
      field('load-architecture').onchange=async e=>{try{const value=await readJson(e.target);if(!value)return;state.architecture=dependencyTools.validateArchitecture(value);state.archFocus=null;editingGroup=null;refreshArchitecture();render();field('architecture-message').textContent='Configuration loaded.'}catch(error){field('architecture-message').textContent=error.message}finally{e.target.value=''}};
      field('save-snapshot').onclick=()=>downloadJson('dependency-snapshot.json',dependencyTools.snapshot(data,field('snapshot-label').value));
      field('load-baseline').onchange=async e=>{try{const value=await readJson(e.target);if(!value)return;state.baseline=dependencyTools.validateSnapshot(value);comparison()}catch(error){field('compare-message').textContent=error.message}finally{e.target.value=''}};
      field('clear-baseline').onclick=()=>{state.baseline=null;field('compare-message').textContent='';comparison()};
      refreshArchitecture();

      async function copyJson(value,messageId,fallbackId){const text=JSON.stringify(value,null,2);try{await navigator.clipboard.writeText(text);field(messageId).textContent='Copied.'}catch{const input=field(fallbackId);input.value=text;input.closest('details').open=true;input.focus();input.select();field(messageId).textContent='Copy the selected JSON.'}}
      field('copy-snapshot').onclick=()=>copyJson(dependencyTools.snapshot(data,field('snapshot-label').value),'compare-message','paste-baseline');
      field('copy-architecture').onclick=()=>copyJson(state.architecture,'architecture-message','paste-architecture');
      field('use-pasted-baseline').onclick=()=>{try{state.baseline=dependencyTools.validateSnapshot(JSON.parse(field('paste-baseline').value));comparison()}catch(error){field('compare-message').textContent=error.message}};
      field('use-pasted-architecture').onclick=()=>{try{state.architecture=dependencyTools.validateArchitecture(JSON.parse(field('paste-architecture').value));state.archFocus=null;editingGroup=null;refreshArchitecture();render();field('architecture-message').textContent='Configuration loaded.'}catch(error){field('architecture-message').textContent=error.message}};
