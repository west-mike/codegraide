// Pure operations shared by the offline explorer and its regression tests.
const dependencyTools = (() => {
  const local = n => n.kind === 'local-file' || n.kind === 'local-module';
  const identity = n => JSON.stringify([n.kind, n.name]);
  function trace(nodes, relations, seeds, direction, depth) {
    const ids = new Set(seeds), adjacent = new Map(nodes.map(n => [n.id, []]));
    for (const r of relations) {
      if (direction !== 'dependents') adjacent.get(r.source)?.push(r.target);
      if (direction !== 'dependencies') adjacent.get(r.target)?.push(r.source);
    }
    let frontier = [...ids];
    for (let step = 0; frontier.length && step < depth; step++) {
      const next = [];
      for (const id of frontier) for (const target of adjacent.get(id) || []) if (!ids.has(target)) { ids.add(target); next.push(target); }
      frontier = next;
    }
    return ids;
  }
  function path(nodes, relations, from, to) {
    const ids = new Set(nodes.filter(local).map(n => n.id));
    if (!ids.has(from) || !ids.has(to)) return null;
    const adjacent = new Map([...ids].map(id => [id, []]));
    for (const r of relations) if (r.kind === 'exact' && ids.has(r.source) && ids.has(r.target)) adjacent.get(r.source).push(r.target);
    const previous = new Map([[from, null]]), queue = [from];
    for (let i = 0; i < queue.length; i++) {
      const id = queue[i];
      if (id === to) { const result = []; for (let p = to; p !== null; p = previous.get(p)) result.push(p); return result.reverse(); }
      for (const target of adjacent.get(id).sort()) if (!previous.has(target)) { previous.set(target, id); queue.push(target); }
    }
    return null;
  }
  function validateArchitecture(value) {
    if (!value || value.format !== 'dependency-architecture-v1' || !Array.isArray(value.groups) || !Array.isArray(value.rules)) throw Error('Choose a dependency architecture configuration.');
    if (value.groups.length > 200 || value.rules.length > 2000) throw Error('Configuration is too large.');
    const names = new Set(), prefixes = new Set();
    const groups = value.groups.map(g => {
      if (typeof g.name !== 'string' || !g.name.trim() || names.has(g.name) || !Array.isArray(g.prefixes) || !g.prefixes.length) throw Error('Each group needs a unique name and at least one path prefix.');
      names.add(g.name);
      const paths = g.prefixes.map(prefix => {
        if (typeof prefix !== 'string') throw Error('Path prefixes must be text.');
        const p = prefix.trim().replaceAll('\\', '/').replace(/\/$/, '');
        if (!p || p.startsWith('/') || p.split('/').includes('..') || prefixes.has(p)) throw Error('Use unique repository-relative path prefixes.');
        prefixes.add(p); return p;
      });
      return { name: g.name, prefixes: paths };
    });
    const rules = value.rules.map(r => { if (!names.has(r.from) || !names.has(r.to) || r.from === r.to) throw Error('Rules need two different existing groups.'); return { from: r.from, to: r.to }; });
    return { format: 'dependency-architecture-v1', groups, rules };
  }
  function membership(nodes, config) {
    const result = new Map();
    for (const n of nodes.filter(local)) {
      const name = n.path || n.name;
      let best = null;
      for (const group of config.groups) for (const prefix of group.prefixes) if ((name === prefix || name.startsWith(prefix + '/')) && (!best || prefix.length > best.length)) best = { name: group.name, length: prefix.length };
      if (best) result.set(n.id, best.name);
    }
    return result;
  }
  function violations(nodes, relations, config) {
    const groups = membership(nodes, config), forbidden = new Set(config.rules.map(r => JSON.stringify([r.from, r.to])));
    return relations.filter(r => forbidden.has(JSON.stringify([groups.get(r.source), groups.get(r.target)]))).map(r => ({ ...r, fromGroup: groups.get(r.source), toGroup: groups.get(r.target), confirmed: r.kind === 'exact' }));
  }
  function snapshot(data, label) {
    return { format: 'dependency-snapshot-v1', label: label.trim() || 'Unlabelled report', nodes: data.nodes.map(n => ({ id: n.id, name: n.name, kind: n.kind, path: n.path })), relations: data.relations.map(r => ({ source: r.source, target: r.target, kind: r.kind })) };
  }
  function validateSnapshot(value) {
    if (!value || value.format !== 'dependency-snapshot-v1' || typeof value.label !== 'string' || !Array.isArray(value.nodes) || !Array.isArray(value.relations)) throw Error('Choose a saved dependency snapshot.');
    if (value.nodes.length > 100000 || value.relations.length > 500000) throw Error('Snapshot is too large.');
    const ids = new Set(), keys = new Set();
    for (const n of value.nodes) { if (typeof n.id !== 'string' || typeof n.name !== 'string' || typeof n.kind !== 'string' || ids.has(n.id) || keys.has(identity(n))) throw Error('Snapshot contains invalid or duplicate nodes.'); ids.add(n.id); keys.add(identity(n)); }
    for (const r of value.relations) if (!ids.has(r.source) || !ids.has(r.target) || typeof r.kind !== 'string') throw Error('Snapshot contains invalid connections.');
    return value;
  }
  function compare(current, baseline) {
    const rows = data => { const nodes = new Map(data.nodes.map(n => [n.id, n])); return new Map(data.relations.map(r => { const source = nodes.get(r.source), target = nodes.get(r.target); return [JSON.stringify([identity(source), identity(target), r.kind]), { source, target, kind: r.kind }]; })); };
    const now = rows(current), before = rows(baseline);
    return { added: [...now].filter(([key]) => !before.has(key)).map(([,r]) => r), removed: [...before].filter(([key]) => !now.has(key)).map(([,r]) => r) };
  }
  function graphWindow(graph, selected, limit) {
    if(graph.items.length<=limit)return {...graph,omitted:0};
    const byId=new Map(graph.items.map(n=>[n.id,n])),adj=new Map(graph.items.map(n=>[n.id,[]]));
    for(const r of graph.relations){adj.get(r.source)?.push(r.target);adj.get(r.target)?.push(r.source)}
    const ids=new Set(),queue=byId.has(selected)?[selected]:[],seeds=graph.items.map(n=>n.id);let cursor=0;
    while(ids.size<limit&&(queue.length||cursor<seeds.length)){
      const id=queue.length?queue.shift():seeds[cursor++];if(ids.has(id)||!byId.has(id))continue;
      ids.add(id);for(const next of adj.get(id)||[])if(!ids.has(next))queue.push(next);
    }
    return {items:graph.items.filter(n=>ids.has(n.id)),relations:graph.relations.filter(r=>ids.has(r.source)&&ids.has(r.target)),omitted:graph.items.length-ids.size};
  }
  return { graphWindow, local, trace, path, validateArchitecture, membership, violations, snapshot, validateSnapshot, compare };
})();
if (typeof module !== 'undefined') module.exports = dependencyTools;
