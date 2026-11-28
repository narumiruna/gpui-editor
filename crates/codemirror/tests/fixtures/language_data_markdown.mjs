import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

assert.equal(process.argv.length, 4, 'usage: node language_data_markdown.mjs REPOSITORY ARCHIVES');
const fixtures = resolve(process.argv[2], 'crates/codemirror/tests/fixtures');
const loadFixture = name => import(pathToFileURL(join(fixtures, name)));
const {pinnedModules} = await loadFixture('pinned_modules.mjs');
const {projectedOffsets} = await loadFixture('editor_line_offsets.mjs');
const root = mkdtempSync(join(realpathSync(tmpdir()), 'language-data-markdown-'));
const emit = (kind, data) => console.log(JSON.stringify({kind, ...data}));
try {
  const p = pinnedModules(resolve(process.argv[3]), root);
  const {languages} = await p.load('@codemirror/language-data');
  const m = await p.load('@codemirror/lang-markdown');
  const h = await p.load('@codemirror/lang-html');
  const j = await p.load('@codemirror/lang-javascript');
  const c = await p.load('@codemirror/lang-css');
  const {Compartment, EditorState, Prec, Transaction} = await p.load('@codemirror/state');
  const {keymap, EditorView, runScopeHandlers} = await p.load('@codemirror/view');
  const {syntaxTree, language, languageDataProp, foldService, foldable, getIndentation, IndentContext, indentService} = await p.load('@codemirror/language');
  const {CompletionContext} = await p.load('@codemirror/autocomplete');
  const {IterMode} = await p.load('@lezer/common');
  const entry = name => languages.find(d => d.name === name);
  const M = await entry('Markdown').load(), H = await entry('HTML').load();
  assert.equal(M, await entry('Markdown').load());
  assert.equal(H, await entry('HTML').load());
  assert.equal(entry('Markdown').support, M);
  assert.notEqual(M.language, m.commonmarkLanguage);
  assert.notEqual(M.language, m.markdownLanguage);
  assert.equal(M.language.data, m.commonmarkLanguage.data);
  assert.equal(M.language.data, m.markdownLanguage.data);
  assert.deepEqual(M.extension, [M.language, M.support]);
  const [html, heading, paste, keys, tag] = M.support;
  const [schema, close, [fixed, local, jsx], css] = html;
  const [oldSchema, oldClose, [oldFixed, oldLocal], oldCss] = H.support;
  assert.equal(m.markdown().support[0], html);
  assert.equal(h.html({matchClosingTags:false}).language.data, H.language.data);
  assert.equal(schema.facet, H.language.data);
  assert.equal(close, h.autoCloseTags);
  assert.equal(close, oldClose);
  assert.equal(close.facet, EditorView.inputHandler);
  assert.notEqual(schema.value.autocomplete, oldSchema.value.autocomplete);
  assert.notEqual(fixed.value.autocomplete, oldFixed.value.autocomplete);
  assert.notEqual(local, oldLocal);
  assert.equal(local.value.autocomplete, oldLocal.value.autocomplete);
  assert.equal(local.value.autocomplete, j.localCompletionSource);
  assert.notEqual(css, oldCss);
  assert.equal(css.value.autocomplete, oldCss.value.autocomplete);
  assert.equal(css.value.autocomplete, c.cssCompletionSource);
  assert.deepEqual(jsx, []);
  assert.equal(paste, m.pasteURLAsLink);
  assert.equal(keys.inner.value, m.markdownKeymap);
  assert.equal(tag.facet, M.language.data);
  const off = m.markdown({addKeymap:false, completeHTMLTags:false, pasteURLAsLink:false});
  assert.deepEqual(off.support, [html, heading]);
  assert(!existsSync(join(root, 'archives')), 'use only local pinned archives');
  emit('identity', {archives:[...p.installed.keys()].sort().map(n => {
    const {version, integrity} = p.pins.get(n); return [n,version,integrity];
  }), support:['html-noMatch-support','heading-fold','URL-paste','high-keymap','Markdown-tag-source'],
    markdownExtent:entry('Markdown').extensions, bindingKeys:m.markdownKeymap.map(k=>k.key)});

  globalThis.document = {body:{style:{zIndex:'', display:'', backgroundColor:'', cssFloat:'', cssText:''}}};
  const ids = new Map([[tag.value.autocomplete,'M-tag'],[schema.value.autocomplete,'M-schema'],
    [oldSchema.value.autocomplete,'H-schema'],[fixed.value.autocomplete,'M-JS-fixed'],
    [oldFixed.value.autocomplete,'H-JS-fixed'],[local.value.autocomplete,'JS-local'],[css.value.autocomplete,'CSS']]);
  const facets = new Map([[M.language.data,'M'],[H.language.data,'H'],[j.javascriptLanguage.data,'JS'],[c.cssLanguage.data,'CSS']]);
  const active = state => state.facet(language) === M.language ? 'M' : state.facet(language) === H.language ? 'H' : null;
  const units = s => s.replace(/\r\n?|\n/g,'\n').length;
  function at(source, token, offset = token.length) {
    const pos = source.indexOf(token);
    assert(pos >= 0, token);
    return units(source.slice(0,pos + offset));
  }
  function facetAt(state, pos, side) {
    let top = syntaxTree(state).topNode;
    for (let n = top; n; n = n.enter(pos,side,IterMode.ExcludeBuffers | IterMode.EnterBracketed))
      if (n.type.isTop) top = n;
    return facets.get(top.type.prop(languageDataProp)) ?? null;
  }
  function result(fn,state,pos,explicit,bytes) {
    syntaxTree(state).resolveInner(0,-1);
    const r = fn(new CompletionContext(state,pos,explicit));
    if (!r) return null;
    let hash = 2166136261;
    for (const {label,type,detail,boost,apply} of r.options) {
      const row = `${label}\0${type ?? ''}\0${detail ?? ''}\0${boost ?? 0}\0${typeof apply === 'function' ? '@snippet' : apply ?? ''}\n`;
      for (const b of Buffer.from(row)) hash = Math.imul(hash ^ b,16777619) >>> 0;
    }
    return {range:[bytes(r.from),bytes(r.to ?? pos)], count:r.options.length, hash,
      labels:r.options.map(o=>o.label).filter(l=>['<span','class','div','display','local','const','--tone'].includes(l)),
      validFor:r.validFor instanceof RegExp ? r.validFor.source : null};
  }
  const source0 = '# Top 😀\r\n\r\n- [x] task\r\n\r\n## Child\r\n\r\nplain 猫 <sp\r\n\r\n```html\r\n<div cl\r\n```\r\n\r\n    <indented\r\n\r\n# Next\r\n\r\n<div cl style="dis" onclick="let local=1; loc">\r\n<style>.a{dis}</style>\r\n<script>let local=1; loc;</script>\r\n</span>\r\n';
  const slot = new Compartment();
  let source = source0, state = EditorState.create({doc:source,extensions:slot.of([])}), retained;
  const exts = {'M':M.extension,'H':H.extension,'bare M':M.language.extension,'high H':Prec.high(H.extension)};
  function snapshot(state,source) {
    const tree = syntaxTree(state), bytes = projectedOffsets(source).byteAt;
    const probes = [['prose','<sp'],['fence','<div cl'],['indented','<indented'],['block attr','<div cl style',7],
      ['CSS attribute start','style="',7],['CSS attribute','style="dis'],['JS attribute','onclick="let local=1; loc'],
      ['CSS raw start','<style>',7],['CSS raw','<style>.a{dis'],['JS raw start','<script>',8],['JS raw','<script>let local=1; loc'],
      ['JS raw end','loc;</script>',4]];
    const queries = probes.map(([label,token,offset])=>{
      const pos = at(source,token,offset);
      return {label, at:bytes(pos), left:tree.resolveInner(pos,-1).name, sides:[-1,1].map(side=>{
        const data = name => state.languageDataAt(name,pos,side);
        const fns = data('autocomplete');
        assert(fns.every(f=>ids.has(f)));
        return {facet:facetAt(state,pos,side),sources:fns.map(f=>ids.get(f)),comment:data('commentTokens'),
          word:data('wordChars'), brackets:data('closeBrackets'),indent:data('indentOnInput').map(r=>r.source),
          results:[false,true].map(explicit=>fns.map(fn=>result(fn,state,pos,explicit,bytes)))};
      })};
    });
    const folds = ['# Top','## Child','# Next'].map(token=>{
      const line=state.doc.lineAt(at(source,token,0)), fold=foldable(state,line.from,line.to);
      return [fold ? [bytes(fold.from),bytes(fold.to)] : null,getIndentation(state,line.from)];
    });
    const pos = at(source,'</span>',3);
    return {active:active(state),root:tree.topNode.name, providers:state.facet(EditorState.languageData).length,
      inputHandlers:state.facet(EditorView.inputHandler).length, folds, queries,
      closing:tree.resolveInner(pos,-1).parent?.name ?? null};
  }
  const compare = (label, extensions) => {
    const s = EditorState.create({doc:source0,extensions});
    const tree = syntaxTree(s).toString();
    const pos = at(source0,'</span>',3);
    emit('choice',{label,task:tree.includes('TaskMarker'),close:syntaxTree(s).resolveInner(pos,-1).parent?.name,
      fences:['<div cl','<indented'].map(t=>facetAt(s,at(source0,t),-1))});
    return tree;
  };
  const stock = compare('catalog',M.extension);
  assert.equal(stock,compare('markdown()',m.markdown().extension));
  assert(!stock.includes('TaskMarker'));
  assert(compare('extended',m.markdown({base:m.markdownLanguage}).extension).includes('TaskMarker'));
  compare('configured fence fallback',m.markdown({defaultCodeLanguage:H}).extension);
  compare('strict HTML',H.extension);
  for (const [step, list, edit] of [['bare',['bare M']],['attach',['M']],['repeat',['M','M']],['select',null],
    ['edit',null,['plain 猫','edited 犬']],['M then H',['M','H']],['high H',['M','high H']],['H then M',['H','M']],
    ['old HTML only',['H']],['restore M',['M']],['detach',[]]]) {
    const before = syntaxTree(state);
    const effects = list?.map(k=>exts[k]);
    const changes = edit && {from:at(source,edit[0],0),to:at(source,edit[0]),insert:edit[1]};
    state = state.update({effects:effects && slot.reconfigure(effects),changes,selection:step === 'select' ? {anchor:4} : undefined}).state;
    if (edit) source = source.replace(...edit);
    // Keep probe labels stable across the same-width text edit.
    assert.equal(state.doc.toString(),source.replace(/\r\n/g,'\n'));
    const sameTree = syntaxTree(state) === before, row = snapshot(state,source);
    assert.equal(sameTree, ['attach','repeat','select','M then H','H then M','old HTML only'].includes(step),step);
    if (step === 'attach' || step === 'repeat') {
      const side = label => row.queries.find(q=>q.label === label).sides[0];
      assert.equal(row.inputHandlers,1);
      assert.equal(side('prose').results[0][0].count,112);
      assert.deepEqual(side('prose').results[0],side('prose').results[1]);
      for (const name of ['fence','indented']) {
        assert.deepEqual(side(name).sources,['M-tag']);
        assert.deepEqual(side(name).results,[[null],[null]]);
      }
      assert.deepEqual(side('block attr').sources,['M-schema']);
      assert.deepEqual(side('JS raw').sources,['M-JS-fixed','JS-local']);
      assert.deepEqual(side('CSS raw').sources,['CSS']);
    }
    if (step === 'attach') retained = {state, source};
    emit('state',{step,source,sameTree,...row});
  }
  emit('state',{step:'retained',source:retained.source,...snapshot(retained.state,retained.source)});

  const quote = '> <section>\r\n>   text 😀\r\n> </section>\r\n';
  const quoted = EditorState.create({doc:quote,extensions:M.extension});
  const bytes = projectedOffsets(quote).byteAt;
  const quoteLine = quoted.doc.line(1), f = foldable(quoted,quoteLine.from,quoteLine.to);
  emit('quote', {source:quote, fold:f && [bytes(f.from),bytes(f.to)], indent:getIndentation(quoted,quoted.doc.line(2).from + 2),
    probes:[['opening overlay',at(quote,'<section>',0)],['removed quote marker',at(quote,'>   text',0)],['segment join',at(quote,'>   text',1)],
      ['start after quote marker',at(quote,'>   text',2)],['inside text',at(quote,'text',2)]]
      .map(([label,pos])=>[label,bytes(pos),[-1,1].map(side=>facetAt(quoted,pos,side))])});

  const nested = '<section>\r\n<div>\r\ntext\r\n</div>\r\n</section>';
  const nestedState = EditorState.create({doc:nested,extensions:M.extension});
  const nestedBytes = projectedOffsets(nested).byteAt;
  emit('mounted-syntax-services',{source:nested,lines:[1,2,3,4,5].map(n=>{
    const line = nestedState.doc.line(n), fold = foldable(nestedState,line.from,line.to);
    return {line:n,indent:getIndentation(nestedState,line.from),fold:fold && [nestedBytes(fold.from),nestedBytes(fold.to)]};
  })});

  const override = foldService.of((_, start, end) => start === nestedState.doc.line(2).from
    ? {from:end - 1, to:end + 1} : null);
  emit('mounted-fold-precedence', {source:nested, folds:[
    [override,M.extension], [M.extension,override], [M.extension],
  ].map(extensions => {
    const state = EditorState.create({doc:nested,extensions});
    return [1,2].map(n => {
      const {from,to} = state.doc.line(n), fold = foldable(state,from,to);
      return fold && [nestedBytes(fold.from),nestedBytes(fold.to)];
    });
  })});

  for (const [htmlSource, targets] of [
    ['<section>\r\n  <div>\r\n    猫</div>\r\n</section>', ['<div>', '    猫', '猫', '</div>', '</section>']],
    ['<div\r\n data-x="😀"\r\n id="y">\r\n  猫\r\n</div>', [' data-x', ' id=', '  猫', '猫', '</div>']],
  ]) {
    const state = EditorState.create({doc:htmlSource,extensions:M.extension});
    const {byteAt} = projectedOffsets(htmlSource);
    const queries = targets.flatMap(target => {
      const pos = state.doc.toString().indexOf(target);
      assert(pos >= 0, target);
      return [[null,false,false], [0,false,false], [state.doc.line(2).from,false,false],
        [null,true,false], [null,true,true]].map(([override, simulate, double]) => {
          const options = {
            overrideIndentation: at => at === override ? 7 : -1,
            simulateBreak: simulate ? pos : undefined,
            simulateDoubleBreak: double,
          };
          return [byteAt(pos), override == null ? null : byteAt(override), simulate, double,
            getIndentation(new IndentContext(state,options),pos)];
        });
    });
    emit('mounted-indent-options',{source:htmlSource,queries});
  }

  const headingSource = '# Heading\r\n\r\ntext 😀\r\n';
  const {byteAt:headingBytes} = projectedOffsets(headingSource);
  const headingOverride = foldService.of((_, start, end) => start === 0 ? {from:end - 1,to:end + 1} : null);
  emit('heading-fold-precedence', {source:headingSource, folds:[
    [headingOverride,M.extension], [M.extension,headingOverride], [M.extension,Prec.high(headingOverride)], [M.extension],
  ].map(extensions => {
    const state = EditorState.create({doc:headingSource,extensions});
    const {from,to} = state.doc.line(1), fold = foldable(state,from,to);
    return fold && [headingBytes(fold.from),headingBytes(fold.to)];
  })});

  const indentOverride = indentService.of((_, pos) => pos === nestedState.doc.line(3).from ? 11 : undefined);
  const indentStop = indentService.of(() => null);
  const indentAt = nestedState.doc.line(3).from;
  emit('mounted-indent-precedence', {source:nested, at:nestedBytes(indentAt), columns:[
    [indentOverride,M.extension], [M.extension,indentOverride], [M.extension,indentStop], [M.extension],
  ].map(extensions => getIndentation(EditorState.create({doc:nested,extensions}),indentAt))});

  const fallback = keymap.of(['Enter','Backspace'].map(key=>({key,run(view){view.fallback = true; return true;}})));
  const pasteFacet = paste.extension.at(-1).facet;
  function input(label, marked, action, extension = M.extension, mode = '') {
    const parts = marked.split('|'), source = parts.join(''), bytes = projectedOffsets(source).byteAt;
    const anchor=units(parts[0]), head=parts.length === 3 ? units(parts[0]+parts[1]) : anchor;
    const state = EditorState.create({doc:source,selection:{anchor,head},extensions:[
      mode === 'highest' ? Prec.highest(fallback) : fallback, extension,
      EditorState.readOnly.of(mode === 'readOnly')]});
    let trs = [], calls = 0;
    const view = {state,composing:mode === 'composition',fallback:false,dispatch(spec){
      trs = Array.isArray(spec) ? spec : [spec instanceof Transaction ? spec : state.update(spec)];
    }};
    const base = () => { calls++; return state.update({...state.replaceSelection(action),userEvent:'input.type',scrollIntoView:true}); };
    const event = {clipboardData:{getData:type=>type === 'text/plain' ? action.slice(6) : ''}};
    const handled = action.startsWith('paste:') ? state.facet(pasteFacet).some(({plugin})=>plugin === paste && plugin.domEventHandlers.paste(event,view)) :
      action === '>' || action === '/' ? state.facet(EditorView.inputHandler).some(fn=>fn(view,anchor,head,action,base)) :
      runScopeHandlers(view,{key:action},'editor');
    let prev = source;
    const transactions = trs.map(tr=>{
      const map = projectedOffsets(prev).byteAt, edits=[];
      tr.changes.iterChanges((from,to,_fromB,_toB,insert)=>edits.push([map(from),map(to),insert.toString()]));
      let buf = Buffer.from(prev);
      for (const [from,to,insert] of edits.slice().reverse()) buf = Buffer.concat([buf.subarray(0,from),Buffer.from(insert),buf.subarray(to)]);
      prev = buf.toString();
      const next = projectedOffsets(prev).byteAt;
      return {edits,doc:prev,selection:[next(tr.state.selection.main.anchor),next(tr.state.selection.main.head),tr.state.selection.main.assoc],
        event:tr.annotation(Transaction.userEvent),scroll:tr.scrollIntoView};
    });
    emit('input',{label,source,selection:[bytes(anchor),bytes(head)],action,mode,handled,fallback:view.fallback,calls,transactions});
  }
  for (const [label,marked,key] of [['continue','😀\r\n\r\n- item|','Enter'],['remove','- |item','Backspace'],
    ['plain Enter','plain|','Enter'],['plain Backspace','plain|','Backspace'],['HTML Enter','<div>plain|</div>','Enter'],
    ['unmounted fence Enter','```html\r\n<div|\r\n```','Enter']]) input(label,marked,key);
  input('highest key wins','- item|','Enter',M.extension,'highest');
  input('no key opt-out','- item|','Enter',off.extension);
  input('inactive Markdown key','- item|','Enter',[H.extension,M.extension]);
  input('plain URL','😀 |Selected 猫| end','paste:www.example.test/猫');
  input('code URL','a `|code|`','paste:https://example.test');
  input('mounted HTML URL','<div>|text|</div>','paste:https://example.test');
  input('selection crosses markup','|plain `code`|','paste:https://example.test');
  input('empty range URL','plain|','paste:https://example.test');
  input('plain clipboard','|Selected|','paste:not a URL');
  input('no paste opt-out','|Selected|','paste:https://example.test',off.extension);
  input('inactive Markdown paste','|Selected|','paste:https://example.test',[H.extension,M.extension]);
  input('read-only URL direct handler','|Selected|','paste:https://example.test',M.extension,'readOnly');
  input('read-only key','- item|','Enter',M.extension,'readOnly');
  input('block HTML >','<!-- 😀 -->\r\n\r\n<section|','>');
  input('block HTML /','<section><|>after','/');
  input('inline incomplete HTML >','prose <section|','>');
  input('fence HTML >','```html\n<section|\n```','>');
  input('HTML read only','<section|','>',M.extension,'readOnly');
  input('HTML composition','<section|','>',M.extension,'composition');
  input('quoted HTML >','> <section|','>');
  input('incomplete raw HTML','<script|','>');
  input('old HTML only','<section|','>',H.extension);
  input('Markdown owns when first','<section|','>',[M.extension,H.extension]);
  input('old HTML owns when first','<section|','>',[H.extension,M.extension]);
} finally {
  rmSync(root,{recursive:true,force:true});
}
