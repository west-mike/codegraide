(() => {
      "use strict";


      const data = JSON.parse(document.getElementById("graph-data").textContent);
      const presentation = data.presentation;
      const svg = document.getElementById("graph");
      const viewport = document.getElementById("viewport");
      const edgeLayer = document.getElementById("edges");
      const nodeLayer = document.getElementById("nodes");
      const details = document.getElementById("details");
      const summary = document.getElementById("summary");
      const breadcrumbs = document.getElementById("breadcrumbs");
      const search = document.getElementById("search");
      document.getElementById('homeButton').onclick=()=>document.querySelector('[data-view="overview"]').click();
      const clearSearch=document.getElementById('clearSearch');
      search.addEventListener('input',()=>{clearSearch.hidden=!search.value});
      clearSearch.onclick=()=>{search.value='';search.dispatchEvent(new Event('input'));search.focus()};
      const emptyGraph = document.getElementById("empty-graph");
      const nodeById = new Map(data.nodes.map(node => [node.id, node]));
      const cycleByNumber = new Map(data.cycles.map(cycle => [cycle.number, cycle]));
      const cycleByMember = new Map(data.cycles.flatMap(cycle => cycle.members.map(id => [id, cycle])));
      const witnessEdges = new Set(data.cycles.flatMap(cycle => cycle.witness_relations.map(relation => `${relation.source}|${relation.target}`)));
      const groupById = new Map(data.hierarchy.map(group => [group.id, group]));
      const directGroupByMember = new Map(data.hierarchy.flatMap(group => group.direct_members.map(id => [id, group])));
      const rootGroups = data.hierarchy.filter(group => !group.parent);
      const outgoing = new Map(data.nodes.map(node => [node.id, []]));
      const incoming = new Map(data.nodes.map(node => [node.id, []]));
      data.relations.forEach(relation => {
        outgoing.get(relation.source)?.push(relation);
        incoming.get(relation.target)?.push(relation);
      });

      const state = {
        mode: "overview",
        direction: 'both', depth: 1, closure: false, topDown: false,
        offsets: {horizontal:new Map(), vertical:new Map()}, history: [], path: null,
        architecture: {format:'dependency-architecture-v1',groups:[],rules:[]}, archFocus:null,
        baseline:null, navPanel:'files',
        currentGroup: rootGroups.length === 1 ? rootGroups[0].id : null,
        selected: null, inspected: null,
        selectedEdge: null,
        hoveredRelation: null,
        query: "",
        visibleKinds: new Set(presentation.local_kind === "local-file" ? ["local-file"] : ["local-module", "local-file", "standard-library", "installed-distribution", "system-header", "external-header", "context-dependent", "ambiguous", "unresolved"]),
        showUncertain: true,
        transform: { x: 30, y: 30, scale: 1 },
        items: [],
        relations: [],
        positions: new Map()
      };

      const kindLabels = {
        "local-module": "Local module",
        "local-file": "Local file",
        "standard-library": "Python standard library",
        "installed-distribution": "Installed package",
        "system-header": "System header",
        "external-header": "External header",
        "context-dependent": "Build-dependent target",
        "ambiguous": "Multiple matches",
        "unresolved": "Unresolved dependency",
        "cycle": "Dependency cycle",
        "architecture": "Architecture group",
        "package": presentation.group_label
      };

      const matchLabels = { exact: "Exact match", inferred: "Likely match", ambiguous: "Multiple matches", unresolved: "Unresolved", "context-dependent": "Build-dependent" };

      const kindColors = {
        "local-module": "var(--local)",
        "local-file": "var(--local)",
        "standard-library": "var(--standard)",
        "installed-distribution": "var(--installed)",
        "system-header": "var(--standard)",
        "external-header": "var(--installed)",
        "context-dependent": "var(--cycle)",
        "ambiguous": "var(--ambiguous)",
        "unresolved": "var(--unresolved)",
        "cycle": "var(--cycle)",
        "package": "var(--border-strong)"
      };

      document.querySelectorAll("[data-kind]").forEach(input => {
        input.checked = state.visibleKinds.has(input.dataset.kind);
      });

      summary.textContent = data.query
        ? `${data.query.label} · ${data.query.found ? `${data.nodes.length} nodes` : "no path found"}`
        : `${data.nodes.length} nodes · ${data.relations.length} relationships · ${data.cycles.length} ${data.cycles.length === 1 ? "cycle" : "cycles"}`;
      if(data.filtered_report)summary.textContent += " · filtered report";
      document.getElementById("module-names").innerHTML = data.nodes
        .filter(node => node.kind === "local-module" || node.kind === "local-file")
        .map(node => `<option value="${escapeHtml(node.name)}"></option>`)
        .join("");

      function escapeHtml(value) {
        return String(value).replace(/[&<>"']/g, character => ({
          "&": "&amp;", "<": "&lt;", ">": "&gt;", "\"": "&quot;", "'": "&#39;"
        })[character]);
      }

      function truncate(value, length) {
        return value.length <= length ? value : `${value.slice(0, length - 1)}…`;
      }

      function visibleNode(node) {
        return state.visibleKinds.has(node.kind);
      }

      function rawVisibleRelations() {
        return data.relations.filter(relation => {
          const source = nodeById.get(relation.source);
          const target = nodeById.get(relation.target);
          return visibleNode(source) && visibleNode(target) && (state.showUncertain || relation.kind === "exact");
        });
      }

      function overviewGraph() {
        const visible = data.nodes.filter(visibleNode);
        const memberToGroup = new Map();
        let items = [];
        const current = state.currentGroup ? groupById.get(state.currentGroup) : null;
        const childGroups = data.hierarchy.filter(group => group.parent === (current?.id || null));

        childGroups.forEach(group => {
          const members = group.members.filter(id => visibleNode(nodeById.get(id)));
          if (!members.length) return;
          items.push({
            id: `package-${group.id}`,
            name: group.qualified_name,
            short_name: group.name,
            subtitle: `${members.length} ${members.length === 1 ? presentation.unit_label : presentation.units_label}`,
            kind: "package",
            packageGroup: group.id,
            members
          });
          members.forEach(id => memberToGroup.set(id, `package-${group.id}`));
        });

        visible.forEach(node => {
          if (node.kind !== "local-module" && node.kind !== "local-file") return;
          const directGroup = directGroupByMember.get(node.id);
          const belongsHere = current ? directGroup?.id === current.id : !directGroup;
          if (belongsHere && !memberToGroup.has(node.id)) {
            const displayNode = node.name === current?.qualified_name
              ? { ...node, short_name: "__init__", subtitle: node.path }
              : node;
            items.push({ ...displayNode, members: [node.id] });
            memberToGroup.set(node.id, node.id);
          }
        });

        visible.filter(node => node.kind !== "local-module" && node.kind !== "local-file").forEach(node => {
          items.push({ ...node, members: [node.id] });
          memberToGroup.set(node.id, node.id);
        });

        data.cycles.forEach(cycle => {
          const members = cycle.members.filter(id => memberToGroup.get(id) === id);
          if (!members.length || members.length !== cycle.members.filter(id => memberToGroup.has(id)).length) return;
          const memberIds = new Set(members);
          items = items.filter(item => !memberIds.has(item.id));
          const cycleItem = {
            id: `cycle-${cycle.number}`,
            name: `Cycle ${cycle.number}`,
            short_name: `Cycle ${cycle.number}`,
            subtitle: `${members.length} ${members.length === 1 ? presentation.unit_label : presentation.units_label}`,
            kind: "cycle",
            cycle: cycle.number,
            members
          };
          items.push(cycleItem);
          members.forEach(id => memberToGroup.set(id, cycleItem.id));
        });

        const combined = new Map();
        rawVisibleRelations().forEach(relation => {
          const source = memberToGroup.get(relation.source);
          const target = memberToGroup.get(relation.target);
          if (!source || !target || source === target) return;
          const key = `${source}|${target}|${relation.kind}`;
          if (!combined.has(key)) combined.set(key, { source, target, kind: relation.kind, originals: [] });
          combined.get(key).originals.push(relation);
        });
        return { items, relations: [...combined.values()] };
      }

      function updateBreadcrumbs() {
        const chain = [];
        let current = state.currentGroup ? groupById.get(state.currentGroup) : null;
        while (current) {
          chain.unshift(current);
          current = current.parent ? groupById.get(current.parent) : null;
        }
        const parts = [{ id: null, name: presentation.root_label }, ...chain.map(group => ({ id: group.id, name: group.name }))];
        breadcrumbs.replaceChildren();
        parts.forEach((part, index) => {
          if (index) {
            const separator = document.createElement("span");
            separator.textContent = "/";
            breadcrumbs.appendChild(separator);
          }
          const button = document.createElement("button");
          button.type = "button";
          button.textContent = part.name;
          button.disabled = part.id === state.currentGroup;
          button.addEventListener("click", () => {
            remember();
            state.currentGroup = part.id;
            state.selected = null;state.inspected = null;
            showPlaceholder();
            render();
          });
          breadcrumbs.appendChild(button);
        });
      }

      function fullGraph() {
        const items = data.nodes.filter(visibleNode).map(node => ({ ...node, members: [node.id] }));
        const ids = new Set(items.map(item => item.id));
        const relations = rawVisibleRelations()
          .filter(relation => ids.has(relation.source) && ids.has(relation.target))
          .map(relation => ({ ...relation, originals: [relation] }));
        return { items, relations };
      }

      function neighborhoodGraph() {
        if (!state.selected) return overviewGraph();
        const seeds = state.selected.startsWith("cycle-")
          ? (cycleByNumber.get(Number(state.selected.slice(6)))?.members || [])
          : [state.selected];
        const allRelations = rawVisibleRelations();
        const ids = dependencyTools.trace(data.nodes, allRelations, seeds, state.direction, state.closure ? Infinity : state.depth);
        const items = data.nodes.filter(node => ids.has(node.id) && visibleNode(node)).map(node => ({ ...node, members: [node.id] }));
        const visibleIds = new Set(items.map(item => item.id));
        const relations = allRelations
          .filter(relation => visibleIds.has(relation.source) && visibleIds.has(relation.target))
          .map(relation => ({ ...relation, originals: [relation] }));
        return { items, relations };
      }

      function graphForMode() {
        if (state.mode === 'architecture') return architectureGraph();
        if (state.mode === 'path') {
          const ids=new Set(state.path || []), pairs=new Set((state.path || []).slice(1).map((id,i)=>`${state.path[i]}|${id}`));
          return {items:data.nodes.filter(n=>ids.has(n.id)).map(n=>({...n,members:[n.id]})),relations:data.relations.filter(r=>r.kind==='exact'&&pairs.has(`${r.source}|${r.target}`))};
        }
        if (state.mode === "full") return fullGraph();
        if (state.mode === "neighborhood") return neighborhoodGraph();
        return overviewGraph();
      }

      function componentId(item) {
        if (item.kind === "cycle") return item.id;
        const cycle = cycleByMember.get(item.id);
        return cycle ? `component-${cycle.number}` : item.id;
      }

      function layout(items, relations) {
        const itemById = new Map(items.map(item => [item.id, item]));
        const components = new Map();
        items.forEach(item => {
          const id = componentId(item);
          if (!components.has(id)) components.set(id, []);
          components.get(id).push(item.id);
        });
        const adjacency = new Map([...components.keys()].map(id => [id, new Set()]));
        const indegree = new Map([...components.keys()].map(id => [id, 0]));
        relations.forEach(relation => {
          const sourceItem = itemById.get(relation.source);
          const targetItem = itemById.get(relation.target);
          if (!sourceItem || !targetItem) return;
          const source = componentId(sourceItem);
          const target = componentId(targetItem);
          if (source !== target && !adjacency.get(source).has(target)) {
            adjacency.get(source).add(target);
            indegree.set(target, indegree.get(target) + 1);
          }
        });

        const rank = new Map([...components.keys()].map(id => [id, 0]));
        const queue = [...components.keys()].filter(id => indegree.get(id) === 0).sort();
        const processed = new Set();
        while (queue.length) {
          const current = queue.shift();
          processed.add(current);
          [...adjacency.get(current)].sort().forEach(target => {
            rank.set(target, Math.max(rank.get(target), rank.get(current) + 1));
            indegree.set(target, indegree.get(target) - 1);
            if (indegree.get(target) === 0) queue.push(target);
          });
          queue.sort();
        }

        const remaining = [...components.keys()].filter(id => !processed.has(id)).sort();
        remaining.forEach(id => {
          const predecessors = relations
            .filter(relation => componentId(itemById.get(relation.target)) === id)
            .map(relation => componentId(itemById.get(relation.source)))
            .filter(source => source !== id && processed.has(source));
          rank.set(id, predecessors.length ? Math.max(...predecessors.map(source => rank.get(source) + 1)) : 0);
        });

        const columns = new Map();
        items.forEach(item => {
          const itemRank = rank.get(componentId(item)) || 0;
          if (!columns.has(itemRank)) columns.set(itemRank, []);
          columns.get(itemRank).push(item);
        });
        columns.forEach(column => column.sort((a, b) => a.name.localeCompare(b.name)));

        const maxRows = Math.max(1, ...[...columns.values()].map(column => column.length));
        const positions = new Map();
        [...columns.entries()].sort((a, b) => a[0] - b[0]).forEach(([columnNumber, column]) => {
          const offset = (maxRows - column.length) * 48;
          column.forEach((item, row) => {
            const base=state.topDown?{x:70+(row-(column.length-1)/2)*250,y:60+columnNumber*160}:{x:70+columnNumber*285,y:60+offset+row*96};
            const moved=state.offsets[state.topDown?'vertical':'horizontal'].get(item.id)||{x:0,y:0};
            positions.set(item.id,{x:base.x+moved.x,y:base.y+moved.y});
          });
        });
        return positions;
      }

      function render(fit=true) {
        const completeGraph = graphForMode();
        const graph = dependencyTools.graphWindow(completeGraph,state.selected,graphLimit);
        graphOmitted=graph.omitted;
        state.items = graph.items;
        state.relations = graph.relations;
        state.positions = layout(graph.items, graph.relations);
        edgeLayer.replaceChildren();
        nodeLayer.replaceChildren();
        emptyGraph.classList.toggle("is-visible", graph.items.length === 0);

        graph.relations.forEach((relation, index) => edgeLayer.appendChild(renderEdge(relation, index)));
        graph.items.forEach(item => nodeLayer.appendChild(renderNode(item)));
        updateSelectionStyles();
        updateViewButtons();
        updateBreadcrumbs();
        refreshNavigator();
        refreshFeatureState();
        if(nodeById.has(state.inspected||state.selected) && state.selectedEdge === null)showNodeDetails(nodeById.get(state.inspected||state.selected));
        if(fit)requestAnimationFrame(fitGraph);
      }

      function renderEdge(relation, index) {
        const source = state.positions.get(relation.source);
        const target = state.positions.get(relation.target);
        const sourceItem = state.items.find(item => item.id === relation.source);
        const targetItem = state.items.find(item => item.id === relation.target);
        const sourceExpanded = sourceItem.kind === "cycle" || sourceItem.kind === "package" || sourceItem.kind === "architecture";
        const targetExpanded = targetItem.kind === "cycle" || targetItem.kind === "package" || targetItem.kind === "architecture";
        const sourceWidth = sourceExpanded ? 220 : 190;
        const targetWidth = targetExpanded ? 220 : 190;
        const sourceHeight = sourceExpanded ? 70 : 58;
        const targetHeight = targetExpanded ? 70 : 58;
        const forward = target.x > source.x;
        const sx = forward ? source.x + sourceWidth : source.x;
        const tx = forward ? target.x : target.x + targetWidth;
        const sy = source.y + sourceHeight / 2;
        const ty = target.y + targetHeight / 2;
        const distance = Math.max(70, Math.abs(tx - sx) * 0.48);
        const c1x = forward ? sx + distance : sx - 70 - (index % 4) * 18;
        const c2x = forward ? tx - distance : tx + 70 + (index % 4) * 18;
        const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
        if(state.topDown){
          const down=target.y>source.y,ax=source.x+sourceWidth/2,bx=target.x+targetWidth/2,ay=source.y+(down?sourceHeight:0),by=target.y+(down?0:targetHeight);
          if(Math.abs(target.y-source.y)<24){const lane=Math.max(source.y+sourceHeight,target.y+targetHeight)+60+(index%4)*16;path.setAttribute('d',`M ${ax} ${source.y+sourceHeight} C ${ax} ${lane}, ${bx} ${lane}, ${bx} ${target.y+targetHeight}`)}
          else {const bend=Math.max(40,Math.abs(by-ay)/2);path.setAttribute('d',`M ${ax} ${ay} C ${ax} ${ay+(down?bend:-bend)}, ${bx} ${by-(down?bend:-bend)}, ${bx} ${by}`)}
        }else path.setAttribute("d", `M ${sx} ${sy} C ${c1x} ${sy}, ${c2x} ${ty}, ${tx} ${ty}`);
        const originals = relation.originals || [relation];
        const witness = originals.some(original => witnessEdges.has(`${original.source}|${original.target}`));
        path.setAttribute("class", `edge${relation.kind === "exact" ? "" : relation.kind === "inferred" ? " inferred" : " uncertain"}${witness ? " is-witness" : ""}`);
        if(dependencyTools.violations(data.nodes,originals,state.architecture).some(r=>r.confirmed))path.classList.add('rule-violation');
        path.dataset.edgeIndex = String(index);
        const title = document.createElementNS("http://www.w3.org/2000/svg", "title");
        title.textContent = `${sourceItem.name} → ${targetItem.name} (${matchLabels[relation.kind] || relation.kind})`;
        path.appendChild(title);
        path.addEventListener("click", event => {
          event.stopPropagation();
          state.selectedEdge = index;
          state.inspected = null;
          showEdgeDetails(relation, sourceItem, targetItem);
          updateSelectionStyles();
        });
        return path;
      }

      function renderNode(item) {
        const position = state.positions.get(item.id);
        const expanded = item.kind === "cycle" || item.kind === "package" || item.kind === "architecture";
        const width = expanded ? 220 : 190;
        const height = expanded ? 70 : 58;
        const group = document.createElementNS("http://www.w3.org/2000/svg", "g");
        group.setAttribute("class", `node ${item.kind}`);
        group.setAttribute("transform", `translate(${position.x} ${position.y})`);
        group.dataset.nodeId = item.id;
        group.setAttribute("role", "button");
        group.setAttribute("tabindex", "0");
        group.setAttribute("aria-label", `${item.name}, ${kindLabels[item.kind]}`);

        const rect = document.createElementNS("http://www.w3.org/2000/svg", "rect");
        rect.setAttribute("width", width);
        rect.setAttribute("height", height);
        rect.setAttribute("rx", "10");
        group.appendChild(rect);

        if (item.kind === "cycle") {
          const mark = document.createElementNS("http://www.w3.org/2000/svg", "path");
          mark.setAttribute("class", "cycle-mark");
          mark.setAttribute("d", "M 18 25 A 10 10 0 1 1 18 45 M 14 25 L 20 25 L 18 19");
          group.appendChild(mark);
        }

        const title = document.createElementNS("http://www.w3.org/2000/svg", "text");
        title.setAttribute("class", "node-title");
        title.setAttribute("x", item.kind === "cycle" ? 38 : 14);
        title.setAttribute("y", expanded ? 29 : 24);
        title.textContent = truncate(item.short_name, 24);
        group.appendChild(title);

        const subtitle = document.createElementNS("http://www.w3.org/2000/svg", "text");
        subtitle.setAttribute("class", "node-subtitle");
        subtitle.setAttribute("x", item.kind === "cycle" ? 38 : 14);
        subtitle.setAttribute("y", expanded ? 49 : 43);
        subtitle.textContent = truncate(item.subtitle, expanded ? 31 : 28);
        group.appendChild(subtitle);

        const tooltip = document.createElementNS("http://www.w3.org/2000/svg", "title");
        tooltip.textContent = item.kind === "cycle"
          ? `${item.name}: ${item.members.map(id => nodeById.get(id).name).join(", ")}`
          : `${item.name}${item.path ? `\n${item.path}` : ""}`;
        group.appendChild(tooltip);

        explorerInteractions.bind(group,{inspect:()=>inspectItem(item),explore:()=>exploreItem(item),blocked:()=>suppressClick});
        bindNodeDrag(group,item);
        return group;
      }

      function inspectItem(item){
        state.inspected=item.id;state.selectedEdge=null;workspace.classList.remove('hide-details');field('toggle-details').textContent='Hide details';field('toggle-details').setAttribute('aria-pressed','true');
        nodeLayer.querySelectorAll('.node').forEach(e=>e.classList.toggle('is-inspected',e.dataset.nodeId===item.id&&item.id!==state.selected));
        if(item.kind==='cycle')showCycleDetails(item);else if(item.kind==='architecture'||item.kind==='package')showArchitectureDetails(item);else showNodeDetails(nodeById.get(item.id));
      }
      function exploreItem(item){
        if(item.kind==='architecture'){remember();state.archFocus=item.archGroup;state.selected=null;state.inspected=null;render();showArchitectureDetails(item)}
        else if(item.kind==='package'){remember();state.currentGroup=item.packageGroup;state.selected=null;state.inspected=null;showPlaceholder();render()}
        else if(item.kind==='cycle'){remember();state.selected=item.members[0];state.inspected=state.selected;state.mode='neighborhood';render()}
        else selectOriginalNode(item.id,true);
      }

      function selectOriginalNode(id, useNeighborhood = false) {
        const node = nodeById.get(id);
        if (!node) return;
        remember();
        state.selected = id;state.inspected=id;
        state.selectedEdge = null;
        if (useNeighborhood) {
          state.mode = "neighborhood";
        } else if (state.mode === "overview") {
          state.currentGroup = directGroupByMember.get(id)?.id || null;
        }
        document.querySelector('[data-view="neighborhood"]').disabled = false;
        render();
        showNodeDetails(node);
      }

      function showNodeDetails(node) {
        if (!node) return;
        const outgoingRelations = outgoing.get(node.id) || [];
        const incomingRelations = incoming.get(node.id) || [];
        const imports = outgoingRelations.map(relation => nodeById.get(relation.target));
        const dependents = incomingRelations.map(relation => nodeById.get(relation.source));
        const cycle = cycleByMember.get(node.id);
        const shown = new Set(state.items.flatMap(item=>item.members));
        const dependencyList = entries => linkList(entries.filter(n=>shown.has(n.id)))+(entries.some(n=>!shown.has(n.id))?`<details><summary>Outside this graph (${entries.filter(n=>!shown.has(n.id)).length})</summary>${linkList(entries.filter(n=>!shown.has(n.id)))}</details>`:'');
        details.innerHTML = `
          <h2>${escapeHtml(node.name)}</h2>
          <div class="kind-label"><span class="kind-swatch" style="--kind-color:${kindColors[node.kind]}"></span>${kindLabels[node.kind]}</div>
          ${node.path ? `<code class="path">${escapeHtml(node.path)}</code>` : ""}
          ${node.version ? `<p>Version ${escapeHtml(node.version)}</p>` : ""}
          ${node.outgoing_dependencies_analyzed === false ? `<p>Dependencies were not analyzed for this file type.</p>` : ""}
          ${node.unresolved_reason ? `<p>Reason: ${escapeHtml(node.unresolved_reason.replaceAll("-", " "))}</p>` : ""}
          ${cycle ? `<p>Member of <strong>Cycle ${cycle.number}</strong>.</p>` : ""}
          ${node.candidates.length ? candidateMarkup(node.candidates) : ""}
          <h3>Dependencies · in graph</h3>${dependencyList(imports)}
          <h3>Used by · in graph</h3>${dependencyList(dependents)}
          ${cycle ? `<button type="button" class="detail-action" id="show-cycle">Show cycle</button>` : ""}
        `;
        bindNodeLinks();
        document.getElementById("show-cycle")?.addEventListener("click", () => {
          selectOriginalNode(node.id, true);
        });
      }

      function showCycleDetails(item) {
        const cycle = cycleByNumber.get(item.cycle);
        const members = cycle.members.map(id => nodeById.get(id));
        const witness = cycle.witness_nodes.map(id => nodeById.get(id)?.name).filter(Boolean);
        details.innerHTML = `
          <h2>Cycle ${cycle.number}</h2>
          <div class="kind-label"><span class="kind-swatch" style="--kind-color:${kindColors.cycle}"></span>${"Dependency cycle"}</div>
          <p class="placeholder">These files or modules form a dependency cycle. Overview shows them as one group.</p>
          <h3>Cycle path</h3><p><code>${escapeHtml(witness.join(" → "))}</code></p><h3>Dependencies to review</h3>${cycle.recommended_cuts.length ? `<ul class="evidence">${cycle.recommended_cuts.map((relation, index) => `<li><button type="button" class="node-link" data-cut="${index}">${escapeHtml(nodeById.get(relation.source)?.name || relation.source)} → ${escapeHtml(nodeById.get(relation.target)?.name || relation.target)}</button><br>${relation.evidence.length} ${relation.evidence.length === 1 ? "reference" : "references"}</li>`).join("")}</ul>` : `<p class="empty-list">No suggestions.</p>`}<p class="placeholder">Removing these dependencies would break this cycle. Review the code before making changes.</p>
          <h3>Members</h3>${linkList(members)}
          <button type="button" class="detail-action" id="expand-cycle">Expand cycle</button>
        `;
        bindNodeLinks();
        details.querySelectorAll("[data-cut]").forEach(button => {
          button.addEventListener("click", () => {
            const relation = cycle.recommended_cuts[Number(button.dataset.cut)];
            showEdgeDetails(relation, nodeById.get(relation.source), nodeById.get(relation.target));
          });
        });
        document.getElementById("expand-cycle").addEventListener("click", () => {
          selectOriginalNode(cycle.members[0], true);
        });
      }

      function showEdgeDetails(relation, sourceItem, targetItem) {
        const originals = relation.originals || [relation];
        const evidence = originals.flatMap(original => original.evidence || []);
        details.innerHTML = `
          <h2>${escapeHtml(sourceItem.name)} → ${escapeHtml(targetItem.name)}</h2>
          <div class="kind-label">${escapeHtml(matchLabels[relation.kind] || relation.kind)}</div>
          ${relation.inference_basis ? `<p>Basis: ${escapeHtml(relation.inference_basis === "unique-repository-suffix" ? "One project file matches the include path" : relation.inference_basis)}.</p>` : ""}
          <h3>References</h3>
          ${evidence.length ? `<ul class="evidence">${evidence.map(item => `<li><code>${escapeHtml(item.source_path)}:${item.line}:${item.column}</code><br>${item.usage === "include" ? "includes" : "imports"} ${escapeHtml(item.import_name)}<br><span class="empty-list">${[item.scope,item.usage,item.requirement,item.conditional?"conditional":null].filter(Boolean).map(escapeHtml).join(" · ")}</span></li>`).join("")}</ul>` : `<p class="empty-list">Connection between groups.</p>`}
        `;
      }

      function candidateMarkup(candidates) {
        return `<h3>Possible targets</h3><div class="link-list">${candidates.map(candidate => `<div>${escapeHtml(candidate.name)}${candidate.detail ? `<br><span class="empty-list">${escapeHtml(candidate.detail)}</span>` : ""}</div>`).join("")}</div>`;
      }

      function linkList(nodes) {
        const unique = [...new Map(nodes.filter(Boolean).map(node => [node.id, node])).values()];
        if (!unique.length) return `<div class="empty-list">None in this graph</div>`;
        return `<div class="link-list">${unique.map(node => `<button type="button" class="node-link" data-select-node="${node.id}">${escapeHtml(node.name)}</button>`).join("")}</div>`;
      }

      function bindNodeLinks() {
        details.querySelectorAll("[data-select-node]").forEach(button => {
          button.addEventListener("click", () => selectOriginalNode(button.dataset.selectNode, true));
        });
      }

      function showPlaceholder() {
        if (data.query) {
          const ordered = data.query.ordered_nodes.map(id => nodeById.get(id)).filter(Boolean);
          details.innerHTML = `
            <h2>${escapeHtml(data.query.label)}</h2>
            <div class="kind-label">Exact local ${data.query.kind === "closure" ? "reachability" : "path"} query</div>
            ${data.query.found ? `<h3>Result</h3>${linkList(ordered)}` : `<p class="empty-list">No path found using exact project dependencies.</p>`}
          `;
          bindNodeLinks();
          return;
        }
        details.innerHTML = `
          <h2>Details</h2>
          <p class="placeholder">Select a file or module to see its dependencies. Select an arrow to see the import or include.</p>
        `;
      }

      function updateSelectionStyles() {
        const selectedItem = state.items.find(item => item.id === state.selected);
        const related = new Set(selectedItem ? [selectedItem.id] : []);
        if (selectedItem) {
          state.relations.forEach(relation => {
            if (relation.source === selectedItem.id) related.add(relation.target);
            if (relation.target === selectedItem.id) related.add(relation.source);
          });
        }
        const query = state.query.trim().toLowerCase();
        nodeLayer.querySelectorAll(".node").forEach(element => {
          const item = state.items.find(candidate => candidate.id === element.dataset.nodeId);
          const memberNames = item.members.map(id => nodeById.get(id)?.name || "").join(" ");
          const matches = query && `${item.name} ${memberNames}`.toLowerCase().includes(query);
          element.classList.toggle("is-selected", item.id === state.selected);
          element.classList.toggle("is-inspected", item.id === state.inspected && item.id !== state.selected);
          element.classList.toggle("is-match", Boolean(matches));
          element.classList.toggle("is-dimmed", Boolean(selectedItem) && state.mode!=='neighborhood' && state.mode!=='path' && !related.has(item.id));
        });
        edgeLayer.querySelectorAll(".edge").forEach(element => {
          const index = Number(element.dataset.edgeIndex);
          const relation = state.relations[index];
          const incident = selectedItem && (relation.source === selectedItem.id || relation.target === selectedItem.id);
          const hovered = state.hoveredRelation && (relation.originals || [relation])
            .some(original => `${original.source}|${original.target}` === state.hoveredRelation);
          element.classList.toggle("is-selected", index === state.selectedEdge || Boolean(incident) || Boolean(hovered));
          element.classList.toggle("is-dimmed", Boolean(selectedItem) && state.mode!=='neighborhood' && state.mode!=='path' && !incident);
        });
      }

      function updateViewButtons() {
        document.querySelectorAll("[data-view]").forEach(button => {
          button.setAttribute("aria-pressed", String(button.dataset.view === state.mode));
        });
      }

      function applyTransform() {
        viewport.setAttribute("transform", `translate(${state.transform.x} ${state.transform.y}) scale(${state.transform.scale})`);
      }

      function fitGraph() {
        if (!state.items.length) return;
        const bounds = state.items.reduce((result, item) => {
          const position = state.positions.get(item.id);
          const expanded = item.kind === "cycle" || item.kind === "package" || item.kind === "architecture";
          const width = expanded ? 220 : 190;
          const height = expanded ? 70 : 58;
          result.minX = Math.min(result.minX, position.x);
          result.minY = Math.min(result.minY, position.y);
          result.maxX = Math.max(result.maxX, position.x + width);
          result.maxY = Math.max(result.maxY, position.y + height);
          return result;
        }, { minX: Infinity, minY: Infinity, maxX: -Infinity, maxY: -Infinity });
        const rect = svg.getBoundingClientRect();
        const graphWidth = Math.max(1, bounds.maxX - bounds.minX);
        const graphHeight = Math.max(1, bounds.maxY - bounds.minY);
        const scale = Math.min(1.35, Math.max(0.18, Math.min((rect.width - 90) / graphWidth, (rect.height - 110) / graphHeight)));
        state.transform.scale = scale;
        state.transform.x = (rect.width - graphWidth * scale) / 2 - bounds.minX * scale;
        state.transform.y = (rect.height - graphHeight * scale) / 2 - bounds.minY * scale;
        applyTransform();
      }

      function zoom(factor, centerX = svg.clientWidth / 2, centerY = svg.clientHeight / 2) {
        const oldScale = state.transform.scale;
        const newScale = Math.min(2.5, Math.max(0.15, oldScale * factor));
        const worldX = (centerX - state.transform.x) / oldScale;
        const worldY = (centerY - state.transform.y) / oldScale;
        state.transform.x = centerX - worldX * newScale;
        state.transform.y = centerY - worldY * newScale;
        state.transform.scale = newScale;
        applyTransform();
      }

      document.querySelectorAll("[data-view]").forEach(button => {
        button.addEventListener("click", () => {
          remember();
          state.mode = button.dataset.view;
          state.selectedEdge = null;
          render();
          if (state.selected && !state.selected.startsWith("cycle-")) showNodeDetails(nodeById.get(state.selected));
        });
      });

      document.querySelectorAll("[data-kind]").forEach(input => {
        input.addEventListener("change", () => {
          if (input.checked) state.visibleKinds.add(input.dataset.kind);
          else state.visibleKinds.delete(input.dataset.kind);
          state.inspected = null;
          state.selectedEdge = null;
          showPlaceholder();
          render();
        });
      });

      document.getElementById("uncertain").addEventListener("change", event => {
        state.showUncertain = event.target.checked;
        state.selectedEdge = null;
        render();
      });

      search.addEventListener("input", () => {
        state.query = search.value;
        updateSelectionStyles();
        const exact = data.nodes.find(node => node.name.toLowerCase() === state.query.trim().toLowerCase());
        if (exact && state.selected !== exact.id) selectOriginalNode(exact.id, true);
      });
      search.addEventListener("keydown", event => {
        if (event.key !== "Enter") return;
        const query = search.value.trim().toLowerCase();
        const match = data.nodes.find(node => node.name.toLowerCase().includes(query));
        if (match) selectOriginalNode(match.id, true);
      });

      let drag = null;
      svg.addEventListener("pointerdown", event => {
        if (event.button !== 0 || event.target.closest(".node") || event.target.closest(".edge")) return;
        drag = { x: event.clientX, y: event.clientY, originX: state.transform.x, originY: state.transform.y };
        svg.setPointerCapture(event.pointerId);
        svg.classList.add("is-panning");
      });
      svg.addEventListener("pointermove", event => {
        if (!drag) return;
        if(Math.abs(event.clientX-drag.x)+Math.abs(event.clientY-drag.y)>3)drag.moved=true;
        state.transform.x = drag.originX + event.clientX - drag.x;
        state.transform.y = drag.originY + event.clientY - drag.y;
        applyTransform();
      });
      svg.addEventListener("pointerup", () => { if(drag?.moved){suppressClick=true;setTimeout(()=>suppressClick=false,0)}drag = null; svg.classList.remove("is-panning"); });
      svg.addEventListener("pointercancel", () => { drag = null; svg.classList.remove("is-panning"); });
      svg.addEventListener("wheel", event => {
        event.preventDefault();
        const rect = svg.getBoundingClientRect();
        zoom(event.deltaY < 0 ? 1.12 : 0.89, event.clientX - rect.left, event.clientY - rect.top);
      }, { passive: false });
      svg.addEventListener("click", () => {
        if(suppressClick)return;
        state.inspected = null;
        state.selectedEdge = null;
        showPlaceholder();
        nodeLayer.querySelectorAll(".is-inspected").forEach(e=>e.classList.remove("is-inspected"));
        updateSelectionStyles();
      });

      document.getElementById("zoom-in").addEventListener("click", () => zoom(1.2));
      document.getElementById("zoom-out").addEventListener("click", () => zoom(0.82));
      document.getElementById("fit").addEventListener("click", fitGraph);
      window.addEventListener("resize", fitGraph);

      __CODEGRAIDE_DEPENDENCY_CONTROLS__

      showPlaceholder();
      render();
    })();
