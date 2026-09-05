/* Source rendering consumes normalized spans and call evidence, never an AST. */
function createCallSource({outgoing,nodes,statusLabels,esc,languages}) {
    // Match source evidence, not every occurrence of a function's name.
    function sourceCalls(source,ownerId,path){
      const marks=new Map(),text=source.lines.join('\n');
      for(const relation of outgoing.get(ownerId)||[])for(const evidence of relation.evidence){
        if(evidence.path!==path||evidence.line<source.start_line||evidence.line>source.end_line||!evidence.callee)continue;
        const row=evidence.line-source.start_line,line=source.lines[row],column=evidence.column-(row===0?(source.start_column||1):1);
        // Analyzer columns are UTF-8 bytes; browser string offsets are UTF-16.
        let offset=0,bytes=0;for(const ch of line){if(bytes>=column)break;bytes+=new TextEncoder().encode(ch).length;offset+=ch.length}
        if(bytes!==column)continue;
        const start=source.lines.slice(0,row).reduce((n,l)=>n+l.length+1,0)+offset;
        if(!text.startsWith(evidence.expression,start))continue;
        const name=evidence.callee.replace(/<.*>$/, ''),escaped=name.replace(/[.*+?^${}()|[\]\\]/g,'\\$&');
        const match=new RegExp('\\b'+escaped+'\\b(?=\\s*(?:<[^;{}]*>)?\\s*\\()').exec(evidence.expression);
        if(!match)continue;
        const prefix=text.slice(0,start+match.index),markRow=prefix.split('\n').length-1,markStart=prefix.length-prefix.lastIndexOf('\n')-1;
        const entries=marks.get(markRow)||[];
        entries.push({start:markStart,end:markStart+name.length,status:relation.status,label:`${nodes.get(relation.target)?.name||name}: ${statusLabels[relation.status]||relation.status}`});marks.set(markRow,entries);
      }
      return marks
    }
    function sourceHtml(source,highlighted=[],ownerId=null,path=null){if(!source)return '<div class="notice">Source is optional. Re-run this HTML report with <code>--include-source</code> to inspect it here.</div>';const highlightLine=languages.highlighter(nodes.get(ownerId)?.language),state={block:false},marks=sourceCalls(source,ownerId,path);return `<pre class="source-view">${source.lines.map((line,i)=>`<span class="code-line${highlighted.includes(source.start_line+i)?' call-site':''}" data-line="${source.start_line+i}"><span class="line-no">${source.start_line+i}</span>${highlightLine(line,state,marks.get(i))}</span>`).join('')}</pre>`}
  return {sourceHtml,sourceCalls};
}
if(typeof module!=='undefined')module.exports=createCallSource;
