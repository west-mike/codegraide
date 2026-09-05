/* Optional presentation adapters. Unknown languages remain readable plain text.
 * These are display hints, never parsing or resolution capabilities. */
const ExplorerLanguages = (() => {
    const cppKeywords=new Set('alignas alignof asm auto break case catch class concept const consteval constexpr constinit const_cast continue co_await co_return co_yield decltype default delete do dynamic_cast else enum explicit export extern false for friend goto if inline mutable namespace new noexcept nullptr operator private protected public register reinterpret_cast requires return sizeof static static_assert static_cast struct switch template this thread_local throw true try typedef typeid typename union unsigned using virtual void volatile while'.split(' '));
    const cppTypes=new Set('bool char char8_t char16_t char32_t double float int long short signed size_t string string_view uint8_t uint16_t uint32_t uint64_t wchar_t'.split(' '));

  const adapters = new Map([
    ['cpp', {syntax:true, keywords:cppKeywords, types:cppTypes, comment:'//', block:true, preprocessor:true, scope:'::'}],
    ['python', {syntax:true, keywords:new Set('and as assert async await break case class continue def del elif else except False finally for from global if import in is lambda match None nonlocal not or pass raise return True try while with yield'.split(' ')), types:new Set(), comment:'#', triple:true, scope:'.'}]
  ]);
  const fallback = {syntax:false, keywords:new Set(), types:new Set(), scope:null};
  const esc = value => String(value??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  function get(language) {return adapters.get(language)||fallback;}
  function highlighter(language) {
    const spec=get(language),keywords=spec.keywords,types=spec.types;
    function token(kind,text){return `<span class="tok-${kind}">${esc(text)}</span>`}
    function highlightLine(line, state, marks = []) {
      if (!spec.syntax) return esc(line);
      if (spec.preprocessor && !state.block && line.trimStart().startsWith('#')) return token('preproc', line);
      let out = '', i = 0;
      while (i < line.length) {
        if (state.block) {
          const end = line.indexOf('*/', i);
          if (end < 0) return out + token('comment', line.slice(i));
          out += token('comment', line.slice(i, end + 2));
          i = end + 2; state.block = false; continue;
        }
        // Existing multiline strings take priority over comment-like text.
        if (state.quote) {
          const end = line.indexOf(state.quote, i);
          if (end < 0) return out + token('string', line.slice(i));
          out += token('string', line.slice(i, end + 3));
          i = end + 3; state.quote = null; continue;
        }
        if (spec.comment && line.startsWith(spec.comment, i)) {
          out += token('comment', line.slice(i)); break;
        }
        if (spec.block && line.startsWith('/*', i)) {
          const end = line.indexOf('*/', i + 2);
          if (end < 0) {state.block = true; out += token('comment', line.slice(i)); break;}
          out += token('comment', line.slice(i, end + 2)); i = end + 2; continue;
        }
        if (spec.triple && (line.startsWith('"""', i) || line.startsWith("'".repeat(3), i))) {
          const quote = line.slice(i, i + 3), end = line.indexOf(quote, i + 3);
          if (end < 0) {state.quote = quote; return out + token('string', line.slice(i));}
          out += token('string', line.slice(i, end + 3)); i = end + 3; continue;
        }
        const ch = line[i];
        if (ch === '"' || ch === "'") {
          let j = i + 1;
          while (j < line.length) {
            if (line[j] === '\\') {j += 2; continue;}
            if (line[j] === ch) {j++; break;}
            j++;
          }
          out += token('string', line.slice(i, j)); i = j; continue;
        }
        if (/[A-Za-z_]/.test(ch)) {
          let j = i + 1;
          while (j < line.length && /[A-Za-z0-9_]/.test(line[j])) j++;
          const word = line.slice(i, j), mark = marks.find(m => m.start === i && m.end === j);
          const html = keywords.has(word) ? token(['true','false','nullptr'].includes(word) ? 'literal' : 'keyword', word)
            : types.has(word) ? token('type', word) : esc(word);
          out += mark ? `<mark class="callee-call status-${esc(mark.status)}" title="${esc(mark.label)}" aria-label="${esc(mark.label)}">${html}</mark>` : html;
          i = j; continue;
        }
        if (/[0-9]/.test(ch)) {
          let j = i + 1;
          while (j < line.length && /[A-Za-z0-9_.'’]/.test(line[j])) j++;
          out += token('number', line.slice(i, j)); i = j; continue;
        }
        out += esc(ch); i++;
      }
      return out;
    }

    return highlightLine;
  }
  function owner(name, language) {
    const spec=get(language);
    if(!spec.scope)return 'Unspecified scope';
    // Python display identities carry a module prefix separated by ::.
    const parts=name.split(spec.scope==='.'?'::':spec.scope);
    if(spec.scope==='.'&&parts.length>1){const symbol=parts.pop().split('.');symbol.pop();return [...parts,...symbol].join('.')||'Global scope';}
    parts.pop();return parts.join(spec.scope)||'Global scope';
  }
  function isSupport(node) {
    return /(^|\/)(tests?|perf|benchmarks?|third_party|third-party|3rdparty|vendor|external)\//.test(node.path||'') ||
      (node.language==='cpp' && /(^|\/)doctest[^/]*\.(h|hpp|cc|cpp)$/.test(node.path||''));
  }
  function readable(value, language) {
    const text=value||'';
    if(text.includes('<anonymous-'))return false;
    if(language!=='cpp')return true;
    return (text.match(/</g)||[]).length===(text.match(/>/g)||[]).length&&!/(^|::)DOCTEST_/.test(text);
  }
  function isDirectChild(parent, child) {
    const separator=get(parent.language).scope;
    if(!separator||parent.language!==child.language)return false;
    const prefix=parent.name+separator;
    return child.name.startsWith(prefix)&&!child.name.slice(prefix.length).includes(separator);
  }
  return {get, highlighter, owner, isSupport, readable, isDirectChild};
})();
if(typeof module!=='undefined')module.exports=ExplorerLanguages;
