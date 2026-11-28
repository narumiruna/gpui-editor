// Run with Node and a directory of local SHA-512-pinned npm archives.
import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, realpathSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {pinnedModules} from './pinned_modules.mjs';
import {projectedOffsets, projectInput} from './editor_line_offsets.mjs';

assert.equal(process.argv.length, 3, 'provide the directory of local npm archives');
const archives = resolve(process.argv[2]);
const scratch = mkdtempSync(join(realpathSync(tmpdir()), 'rust-configured-indent.'));
try {
const {load, pins, installed} = pinnedModules(archives, scratch);
// Do not let the shared loader download a missing archive.
for (const {name, version} of pins.values()) {
  const path = join(archives, name.replace('/', '__'), `${name.split('/').at(-1)}-${version}.tgz`);
  assert(existsSync(path), `missing offline archive: ${path}`);
}
const {rustLanguage: stock} = await load('@codemirror/lang-rust');
const {markdown} = await load('@codemirror/lang-markdown');
const {parser: grammar} = await load('@lezer/rust');
const {NodeProp} = await load('@lezer/common');
const {EditorState, Compartment, Transaction, StateField} = await load('@codemirror/state');
const {getIndentation, IndentContext, indentNodeProp, indentService, indentUnit, indentOnInput, foldNodeProp, languageDataProp, syntaxTree} = await load('@codemirror/language');
const emit = value => console.log(JSON.stringify(value));
const rows = stock.parser.nodeSet.types;
const ruleClasses = [[64], [7, 35], [182, 206], [45, 115, 165, 172, 173, 175, 178, 179, 183, 207, 212, 214, 218, 220, 222, 223, 225, 226, 227, 235, 236, 237]];
const stockRules = ruleClasses.map(ids => rows[ids[0]].prop(indentNodeProp));
const inventory = rows.filter(r => r.prop(indentNodeProp)).map(r => [r.id, r.name, stockRules.indexOf(r.prop(indentNodeProp))]);
assert.deepEqual(inventory.map(r => r[0]), ruleClasses.flat().sort((a, b) => a - b));
assert(inventory.every(([id, , cls]) => ruleClasses[cls].includes(id)));
assert(rows.every(r => grammar.nodeSet.types[r.id].prop(indentNodeProp) === undefined));
emit({kind:'pins', modules:[...installed.keys()].map(n => [n,pins.get(n).version,pins.get(n).integrity])});
emit({kind:'inventory', rows:inventory, skipped:rows.filter(r => r.isSkipped).map(r => [r.id,r.name]), closedBy:rows.filter(r => r.prop(NodeProp.closedBy)).map(r => [r.id,r.name,r.prop(NodeProp.closedBy)])});

const unit = '\t', tab = 4;
const contextExts = [indentUnit.of(unit), EditorState.tabSize.of(tab)];
const ticks = StateField.define({create:() => 0, update:(n,tr) => n + +tr.docChanged});
const create = (text, language, exts = []) => EditorState.create({doc:text, extensions:[language, ...contextExts, ...exts]});
const normalize = text => text.replace(/\r\n?/g,'\n');
const size = text => Buffer.byteLength(text);
function byteFn(text) { return projectedOffsets(text).byteAt; }
function point(marked) {
  const [before, after] = marked.split('«');
  assert(after !== undefined && !after.includes('«'));
  const q = normalize(before).length, pos = size(before);
  const raw = before + after;
  assert.equal(byteFn(raw)(q), pos);
  assert(!(before.endsWith('\r') && after.startsWith('\n')));
  return {raw, q, pos};
}
function span(n,b) { return n && [n.type.id,n.name,b(n.from),b(n.to)]; }
function syntax(state, b) {
  const cursor = syntaxTree(state).cursor();
  const out = [];
  do { out.push(span(cursor,b)); } while (cursor.next());
  return out;
}
function config(base, props, calls, rawOf) {
  let configured;
  const strategy = (mode, id) => cx => {
    const b = byteFn(rawOf(cx.state));
    const line = bias => {const l = cx.lineAt(cx.pos,bias); return [b(l.from),l.text];};
    assert.equal(cx.node.type, configured.parser.nodeSet.types[id]);
    calls.push({node:span(cx.node,b), parent:span(cx.node.parent,b), children:Array.from(function*(){for(let c=cx.node.firstChild;c;c=c.nextSibling)yield span(c,b);}()), pos:b(cx.pos), head:b(cx.state.selection.main.head), unit:cx.unit, tab:cx.state.tabSize, before:line(-1), after:line(1), textAfter:cx.textAfter, base:cx.baseIndent, simulated:cx.simulatedBreak === null ? null : b(cx.simulatedBreak), column:cx.column(cx.pos), ticks:cx.state.field(ticks,false) ?? null});
    if (mode === 'stop') return null;
    if (mode === 'continue') return cx.continue();
    if (mode === 'twice') {const first=cx.continue(), second=cx.continue(); return first === null || second === null ? null : first + second + 1;}
    if (mode === 'context') return cx.baseIndent + cx.unit + b(cx.state.selection.main.head) % 2;
    return Number(mode);
  };
  const map = new Map(props);
  configured = base.configure({props:[indentNodeProp.add(r => {
    if (!map.has(r.id)) return undefined;
    const mode=map.get(r.id);
    return mode === null ? null : strategy(mode,r.id);
  })]});
  for (const [id, mode] of props) if (mode === null) assert.equal(configured.parser.nodeSet.types[id].prop(indentNodeProp),null);
  return configured;
}
function query(caseName, marked, props = [], opt = {}) {
  const {raw,q,pos} = point(marked), b = byteFn(raw);
  const calls = [];
  let language = stock;
  if (opt.closedByEmpty) language = language.configure({props:[NodeProp.closedBy.add(r => r.id === 48 ? [] : undefined)]});
  if (opt.unrelated) language = language.configure({props:[foldNodeProp.add(r => r.id === 43 ? null : undefined), NodeProp.group.add(r => r.id === 226 ? [] : undefined)]});
  if (opt.before) language = config(language,opt.before,calls,() => raw);
  if (props.length) language = config(language,props,calls,() => raw);
  if (opt.strict) language = language.configure({strict:true});
  if (opt.quiet) language = language.configure({props:[languageDataProp.add(r => r.isTop ? null : undefined)]});
  // do not change callback row identity after setup in these queries
  const exts = opt.service ? [indentService.of(() => opt.service === 'stop' ? null : +opt.service)] : [];
  let st = create(raw,language,exts);
  if (opt.select) {
    const head = normalize(raw.slice(0, raw.indexOf(opt.select))).length;
    st = st.update({selection:{anchor:head}}).state;
  }
  const provided = opt.override === undefined ? undefined : start => b(start) === (opt.overrideAtQuery ? pos : 0) ? opt.override : -1;
  const options = {simulateBreak: opt.sim ? q : undefined,simulateDoubleBreak:opt.sim === 2,overrideIndentation:provided};
  const run = state => getIndentation(new IndentContext(state,options),q);
  const ast = syntaxTree(st);
  const inner = ast.resolveInner(q,-1).resolve(q,0).enterUnfinishedNodesBefore(q);
  const stockResult = run(create(raw,stock));
  const result = run(st);
  emit({kind:'query',case:caseName,raw,pos,pos16:q,head:b(st.selection.main.head),props,opt,tree:syntax(st,b),inner:span(inner,b),stock:stockResult,result,calls});
}

const block='// 🦀\r\nfn λ() {\r\n«  let x = "字😀";\r}\r\n';
query('block-custom',block,[[43,'17']]);
query('block-strict',block,[[43,'17']],{strict:true});
query('block-root-data-disabled',block,[[43,'17']],{quiet:true});
query('block-stop',block,[[43,'stop'],[8,'19']]);
query('block-disabled-after-custom',block,[[43,null],[8,'19']],{before:[[43,'17']]});
query('block-service-column',block,[[43,'17']],{service:'31'});
query('block-service-inherit',block,[[43,'17']],{service:'stop'});
query('block-selected',block,[[43,'context']],{select:' ='});
query('top-continue-zero','«fn f() {}',[[8,'continue']]);
query('nested-twice','fn f() {\n  if yes {\n«    yes();\n  }\n}',[[43,'twice'],[64,'continue'],[8,'5']]);
query('statement-continue','fn f() {\n  let x =\n«    1;\n}',[[226,'continue'],[43,'11']]);
query('statement-disabled','fn f() {\n  let x =\n«    1;\n}',[[226,null]]);
query('statement-stock-unrelated','fn f() {\n  let x =\n«    1;\n}',[],{unrelated:true});
query('if-brace','fn f() {\n  if yes\n«  { g(); }\n}',[]);
query('if-brace-disabled','fn f() {\n  if yes\n«  { g(); }\n}',[[64,null]]);
query('if-else','fn f() {\n  if yes { g(); }\n«  else { h(); }\n}',[]);
query('if-else-disabled','fn f() {\n  if yes { g(); }\n«  else { h(); }\n}',[[64,null]]);
query('if-continued','fn f() {\n  if yes &&\n«    no { g(); }\n}',[]);
query('match-disabled','fn f() {\n  match x {\n    _ =>\n«      1,\n  }\n}',[[165,null]]);
query('match-custom','fn f() {\n  match x {\n    _ =>\n«      1,\n  }\n}',[[165,'21']]);
query('match-first-term','fn f() {\n  match x {\n    0 =>\n«      1,\n    _ => 2\n  }\n}',[[115,'21'],[165,'23']]);
query('attribute-foreign-term','extern "C" {\n  #[x]\n«  safe fn g();\n}',[[182,'14'],[206,'16']]);
query('attribute-stock','fn f() {\n  #[cfg(\n«    unix)]\n  fn g() {}\n}',[]);
query('attribute-stock-continue','fn f() {\n  #[x]\n«  fn g() {}\n}',[[43,'13']]);
query('attribute-custom','fn f() {\n  #[x]\n«  fn g() {}\n}',[[182,'14'],[43,'13']]);
query('attribute-disabled','fn f() {\n  #[x]\n«  fn g() {}\n}',[[182,null],[43,'13']]);
query('comment-disabled','fn f() {\r\n  /* 🦀\r\n«    😀 text */\r\n}',[[7,null]]);
query('comment-continue','fn f() {\r\n  /* 🦀\r\n«    😀 text */\r\n}',[[7,'continue'],[43,'9']]);
query('string-disabled','fn f() {\n  let x = "a\n«   字😀";\n}',[[35,null]]);
query('string-stop','fn f() {\n  let x = "a\n«   字😀";\n}',[[35,'stop'],[226,'9']]);
query('aligned','fn f() {\n\tcall(   1,\n« 2);\n}',[]);
query('aligned-disabled','fn f() {\n\tcall(   1,\n« 2);\n}',[[141,null]],{before:[[141,'17']]});
query('line-comment-not-anchor','fn f() {\n\tcall( // 🦀\n« 2);\n}',[]);
query('block-comment-not-anchor','fn f() {\n\tcall( /* 🦀 */\n« 2);\n}',[]);
query('block-comment-before-content','fn f() {\n\tcall( /* 🦀 */ 1,\n« 2);\n}',[]);
query('error-not-anchor','fn f() {\n\tg( @\n« 1);\n}',[]);
query('closed-by-empty-scout','fn f() {\n\tcall(  1\n« );\n}',[[141,null]],{closedByEmpty:true});
query('angle-list-is-not-delimited','fn f() {\n  let a: Foo<\n«    i32\n  > = b;\n}',[]);
query('param-term-194','fn f(\n« x: i32\n) {}',[[156,'18'],[194,'28']]);
query('param-term-156','fn f() {\n  let t = move |\n« x: i32\n  | x;\n}',[[156,'18'],[194,'28']]);
query('single-break','\tfn λ() {«  }\r\n',[[43,'context']],{sim:1,override:7});
query('double-break','\tfn λ() {«  }\r\n',[[43,'context']],{sim:2,override:7});
query('sim-override-site','\tfn λ() {«  }\r\n',[[43,'context']],{sim:1,override:7,overrideAtQuery:true});
query('base-skips-sibling-line','\tfn f() {\r\n\t/* start\r end */ call(\r\n«  1);\r\n}',[[141,'context']],{select:'call'});
query('lone-cr-stock-scout','fn f() {\r\tcall(\r« 1);\r}',[]);
query('lone-cr-unrelated','fn f() {\r\tcall(\r« 1);\r}',[],{unrelated:true});
query('lone-cr-unused-indent','fn f() {\r\tcall(\r« 1);\r}',[[115,'21']]);
query('lone-cr-callback','fn f() {\r\tcall(\r« 1);\r}',[[141,'context']]);
query('zero-break-scout','«fn f() {}',[[8,'context']],{sim:1});
query('offset-after-emoji','fn f() { \t"🦀";« f(); }\r\n',[[43,'context']],{select:'f();'});
query('utf16-100-vs-scalars','fn f() { /* x«' + '😀'.repeat(60) + 'abc */ }',[[7,'context']]);
query('unfinished-eof','fn f() {\r\n  call(1,\r\n«',[[141,'17'],[43,'19']]);
query('unfinished-eof-disabled','fn f() {\r\n  call(1,\r\n«',[[141,null],[43,'19']]);
query('recovery-scout','fn f() {\r\n  @\r\n«  let x = 1;\r\n}',[[43,'17']]);

for (const quoted of [false,true]) for (const fallback of [false,true]) {
  const quotedPrefix = quoted ? '> ' : '';
  const marked = `head\r\n\r\n${quotedPrefix}\`\`\`${fallback?'other':'rr'}\r\n${quotedPrefix}fn λ() {\r\n${quotedPrefix}«  let a = "😀";\r\n${quotedPrefix}}\r\n${quotedPrefix}\`\`\`\r\n`;
  const {raw,q,pos} = point(marked), calls = [], b = byteFn(raw);
  const language = config(stock,[[43,'17']],calls,() => raw);
  const state = create(raw,markdown({codeLanguages:name => name==='rr' ? language : null,defaultCodeLanguage:fallback ? language : undefined,addKeymap:false}));
  const result = getIndentation(state,q);
  assert.equal(result,17);
  emit({kind:'markdown',case:`${quoted?'quote':'plain'}-${fallback?'fallback':'named'}`,raw,pos,props:[[43,'17']],result,calls,inner:span(syntaxTree(state).resolveInner(q,0),b)});
}

{
  const {raw,q,pos} = point('```rr\r\nfn λ() {\r\n«  call("😀");\r\n}\r\n```\r\n');
  const calls = [], props = [[43,'continue'],[220,'continue'],[8,'continue']];
  const language = config(stock,props,calls,() => raw);
  const state = create(raw,markdown({codeLanguages:() => language,addKeymap:false}));
  const result = getIndentation(state,q);
  assert.deepEqual(calls.map(call => call.node[0]), [43,220,8]);
  emit({kind:'markdown',case:'continue-through-root',raw,pos,props,result,calls});
}

const inputMarked='// 🦀\r\nfn λ() {\r\n    «\r\n';
for (const mode of ['17','stop','reconfigure','strict-repair','strict-reject','starting-data-only','starting-data-disabled','reconfigure-service','disabled-if']) {
  const marked = mode === 'disabled-if' ? 'fn f() {\n  if yes\n        «\n    g();\n  }\n}' : inputMarked;
  const {raw,q,pos} = point(marked), calls = [];
  const insert = ['strict-reject','disabled-if'].includes(mode) ? '{' : '}';
  const interim = Buffer.concat([Buffer.from(raw).subarray(0,pos),Buffer.from(insert),Buffer.from(raw).subarray(pos)]).toString();
  const props = mode === 'disabled-if' ? [[64,null]] : [[43,mode==='stop'?'stop':'17']];
  const watch = config(stock,props,calls,() => interim);
  const strict = watch.configure({strict:true});
  // Strict editor failure is a native safety policy; compare strict parsers
  // directly for the negative case instead of depending on JS state laziness.
  if (mode === 'strict-reject') {
    let strictError;
    assert.throws(() => stock.parser.configure({strict:true}).parse(normalize(interim)), e => {
      assert(e instanceof SyntaxError);
      const offset16 = Number(e.message.match(/at (\d+)$/)?.[1]);
      strictError = {message:e.message,offset16,byte:byteFn(interim)(offset16)};
      return true;
    });
    emit({kind:'input',case:mode,raw,pos,insert,interim,result:null,calls,strictError});
    continue;
  }
  if (mode === 'strict-repair') assert.doesNotThrow(() => stock.parser.configure({strict:true}).parse(normalize(interim)));
  const slot = new Compartment();
  const quiet = stock.configure({props:[languageDataProp.add(r => r.isTop ? null : undefined)]});
  const start = mode === 'reconfigure' || mode === 'reconfigure-service' ? stock : mode === 'starting-data-disabled' ? quiet : watch;
  const state = create(raw,slot.of(start),[ticks,indentOnInput()]).update({selection:{anchor:q}}).state;
  const event = mode === '17' ? 'input.complete' : 'input.type';
  const tr = state.update({changes:{from:q,insert},selection:{anchor:q+insert.length},effects:mode==='starting-data-disabled' ? [slot.reconfigure(watch)] : mode==='reconfigure-service' ? [slot.reconfigure([watch, indentService.of(() => 5)])] : mode==='reconfigure' ? [slot.reconfigure([watch,indentUnit.of('   ')])] : mode==='starting-data-only' ? [slot.reconfigure([])] : mode==='strict-repair' ? [slot.reconfigure(strict)] : [],annotations:Transaction.userEvent.of(event)});
  const result = projectInput(raw,tr);
  emit({kind:'input',case:mode,raw,pos,insert,interim,result,head:byteFn(result)(tr.newSelection.main.head),event,calls});
}
for (const mode of ['17', 'stop', 'continue']) {
  const {raw,q,pos} = point('Before.\r\n\r\n```rr\r\n// 字😀\r\nfn λ() {\r\n    «\r\n```\r\n');
  const props = mode === 'continue' ? [[43,mode],[220,'9']] : [[43,mode]];
  const interim = Buffer.from(raw).subarray(0,pos).toString() + '}' + Buffer.from(raw).subarray(pos).toString();
  const calls = [], language = config(stock,props,calls,() => interim);
  const support = markdown({codeLanguages:name => name === 'rr' ? language : null,addKeymap:false});
  const state = create(raw,support,[indentOnInput()]).update({selection:{anchor:q}}).state;
  const tr = state.update({changes:{from:q,insert:'}'},selection:{anchor:q+1},annotations:Transaction.userEvent.of('input.type')});
  const result = projectInput(raw,tr);
  assert.equal(calls.length, mode === 'continue' ? 2 : 1);
  assert(result.includes(({17:'\t\t\t\t ',stop:'    ',continue:'\t\t '})[mode] + '}\r\n```'));
  emit({kind:'markdown-input',case:mode,raw,pos,props,result,head:byteFn(result)(tr.newSelection.main.head),calls});
}
} finally {
  rmSync(scratch, {recursive:true, force:true});
}
