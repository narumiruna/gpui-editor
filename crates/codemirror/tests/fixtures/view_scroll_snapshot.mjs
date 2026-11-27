// Run as a browser module with the integrity-checked upstream import map.
import {EditorSelection} from '@codemirror/state';
import {EditorView} from '@codemirror/view';

const encode = new TextEncoder();
const bytes = (doc, pos) => encode.encode(doc.slice(0, pos)).length;
const documentText = rows => Array.from({length: rows}, (_, i) =>
  `row${i} a😀β ${'line '.repeat(24)}`).join('\n');
const offset = ({scrollLeft: x, scrollTop: y}) => ({x, y});
const rect = ({left, top, right, bottom}) => ({left, top, right, bottom});
const block = (doc, {from, top}) => ({from: bytes(doc, from), top});

function mount(doc, options = {}) {
  const {rtl, scale = [1, 1], hiddenX, padding = 4, covered, scrollTo} = options;
  const parent = document.body.appendChild(document.createElement('div'));
  parent.style.cssText = 'width:270px;height:180px;overflow:auto;position:relative';
  const height = parent.appendChild(document.createElement('div'));
  height.style.height = '1400px';
  parent.scrollTop = 90;
  const extensions = covered ? [EditorView.scrollMargins.of(() => covered)] : [];
  const view = new EditorView({parent, doc, extensions, scrollTo});
  view.dom.dir = rtl ? 'rtl' : 'ltr';
  view.scrollDOM.style.cssText = `position:fixed;left:40px;top:50px;width:180px;height:100px;`
    + `overflow-x:${hiddenX ? 'hidden' : 'auto'};overflow-y:auto;transform-origin:top left;`
    + `transform:scale(${scale.join(',')})`;
  view.contentDOM.style.cssText = `padding-top:${padding}px;font:16px/20px monospace;`;
  return {view, parent, destroy() { view.destroy(); parent.remove(); }};
}

// Record assignments in unscaled scroller units before Chrome clamps or rounds.
function trace(view, run) {
  const scroll = view.scrollDOM, before = offset(scroll), assigned = {};
  for (const [axis, property] of [['x', 'scrollLeft'], ['y', 'scrollTop']]) {
    const {get, set} = Object.getOwnPropertyDescriptor(Element.prototype, property);
    Object.defineProperty(scroll, property, {
      configurable: true,
      get() { return get.call(this); },
      set(value) { assigned[axis] = value; set.call(this, value); }
    });
  }
  try {
    run();
  } finally {
    delete scroll.scrollLeft;
    delete scroll.scrollTop;
  }
  return {before, assigned, actual: offset(scroll)};
}

function snapshot(name, options = {}) {
  const {rows = 60, scroll = {x: 24, y: 179}, shift, edit, restoreRows} = options;
  const doc = documentText(rows), host = mount(doc, options), {view} = host;
  view.scrollDOM.scrollLeft = scroll.x;
  view.scrollDOM.scrollTop = scroll.y;
  view.measure();
  const state = view.viewState;
  if (shift != null) view.scrollDOM.scrollTop = state.viewportLines[0].top - shift;
  const current = offset(view.scrollDOM), viewportFrom = bytes(doc, state.viewport.from);
  const firstViewportBlock = block(doc, state.viewportLines[0]);
  let probe;
  const lookup = state.lineBlockAtHeight;
  state.lineBlockAtHeight = function(height) {
    const result = lookup.call(this, height);
    probe = {height, block: block(doc, result)};
    return result;
  };
  let effect = view.scrollSnapshot();
  state.lineBlockAtHeight = lookup;
  const anchor = bytes(doc, effect.value.range.head);
  const geometry = {offset: current, paddingTop: state.paddingTop, viewportFrom, firstViewportBlock};
  const changes = edit ? edit(effect.value.range.head) : [];
  const edits = changes.map(({from, to = from, insert}) =>
    ({from: bytes(doc, from), to: bytes(doc, to), insert}));
  if (changes.length) {
    effect = effect.map(view.state.changes(changes));
    view.dispatch({changes});
    view.measure();
  }
  const targetHost = restoreRows ? mount(documentText(restoreRows), {...options, scrollTo: effect}) : host;
  const targetView = targetHost.view;
  if (!restoreRows) {
    targetView.scrollDOM.scrollTop = 0;
    targetView.scrollDOM.scrollLeft = 0;
    targetView.measure();
  }
  const targetDoc = targetView.state.doc.toString();
  const mapped = bytes(restoreRows ? doc : targetDoc, effect.value.range.head);
  const clipped = Math.min(effect.value.range.head, targetDoc.length);
  // Trace just the snapshot, not incidental writes from the measure loop.
  const scrollIntoView = targetView.docView.scrollIntoView;
  let top, observed;
  targetView.docView.scrollIntoView = function(target) {
    if (!target.isSnapshot || target.range.head !== clipped) throw Error('expected clipped snapshot');
    top = targetView.viewState.lineBlockAt(target.range.head).top;
    observed = trace(targetView, () => scrollIntoView.call(this, target));
  };
  const parentBefore = targetHost.parent.scrollTop;
  if (!restoreRows) targetView.dispatch({effects: effect});
  targetView.measure();
  if (!observed || parentBefore !== targetHost.parent.scrollTop) throw Error('snapshot did not restore only its scroller');
  const restore = {rows: restoreRows, anchor: bytes(targetDoc, clipped), top, ...observed};
  if (host !== targetHost) targetHost.destroy();
  host.destroy();
  return {kind: 'snapshot', name, rows, geometry, probe, anchor, edits, mapped, restore};
}

function reveal(name, options) {
  const rows = 80, doc = documentText(rows), host = mount(doc, options);
  const {view, parent} = host;
  const {rtl, scale = [1, 1], covered, hiddenX, x, y, xMargin = 5, yMargin = 5,
    head, anchor = head, assoc = 0} = options;
  const scroll = view.scrollDOM;
  scroll.scrollLeft = rtl ? -70 : 70;
  scroll.scrollTop = 5 * 20;
  view.measure();
  const position = ([line, col]) => view.state.doc.line(line).from + col;
  const [a, h] = [position(anchor), position(head)];
  const range = a === h ? EditorSelection.cursor(h, assoc) : EditorSelection.range(a, h);
  const effect = EditorView.scrollIntoView(range, {x, y, xMargin, yMargin});
  const css = scroll.getBoundingClientRect();
  const clip = {left: css.left, top: css.top, right: css.left + scroll.clientWidth * scale[0],
    bottom: css.top + scroll.clientHeight * scale[1]};
  if (covered) {
    const bounds = view.viewState.scrollParent.getBoundingClientRect();
    clip.left = Math.max(clip.left, Math.max(bounds.left, 0) + covered.left);
    clip.right = Math.min(clip.right, Math.min(bounds.right, innerWidth) - covered.right);
    clip.top = Math.max(clip.top, Math.max(bounds.top, 0) + covered.top);
    clip.bottom = Math.min(clip.bottom, Math.min(bounds.bottom, innerHeight) - covered.bottom);
  }
  const scroller = {clip, offsetWidth: scroll.offsetWidth, offsetHeight: scroll.offsetHeight,
    scaleX: scale[0], scaleY: scale[1], scrollX: !hiddenX && scroll.scrollWidth > scroll.clientWidth,
    scrollY: scroll.scrollHeight > scroll.clientHeight, direction: rtl ? 'rtl' : 'ltr'};
  const coords = view.docView.coordsAt;
  const measurements = [];
  view.docView.coordsAt = function(pos, side) {
    const result = coords.call(this, pos, side);
    measurements.push({pos: bytes(doc, pos), side, rect: result ? rect(result) : null});
    return result;
  };
  const parentBefore = parent.scrollTop;
  const observed = trace(view, () => view.docView.scrollIntoView(effect.value));
  if (parentBefore !== parent.scrollTop) throw Error('fixed scroller moved a parent');
  host.destroy();
  const target = {anchor: bytes(doc, a), head: bytes(doc, h), assoc, x, y, xMargin, yMargin};
  return {kind: 'reveal', name, rows, target, scroller, measurements, ...observed};
}

try {
  const cases = [
    snapshot('padded anchor and positive x', {padding: 14}),
    snapshot('prepend Unicode lines and restore negative RTL x', {
      rtl: true, scroll: {x: -24, y: 179}, edit: () => [{from: 0, insert: '😀 prepended\nmore\n'}]
    }),
    snapshot('insert exactly at anchor', {edit: from => [{from, insert: '字 inserted\n'}]}),
    snapshot('delete across anchor', {edit: at => [{from: at - 2, to: at + 3, insert: ''}]}),
    snapshot('viewport fallback at 200 CSS px', {rows: 400, scroll: {x: 24, y: 5600}, shift: 200}),
    snapshot('height lookup wins past 200 CSS px', {rows: 400, scroll: {x: 24, y: 5600}, shift: 201}),
    snapshot('constructor restores and clips to end', {restoreRows: 4}),
    reveal('visible nearest', {head: [9, 20], x: 'nearest', y: 'nearest'}),
    reveal('nearest forward oversized range', {anchor: [5, 2], head: [15, 60], x: 'nearest', y: 'nearest'}),
    reveal('nearest backward oversized range', {anchor: [15, 60], head: [5, 2], x: 'nearest', y: 'nearest'}),
    reveal('nearest backward when start inside clip', {anchor: [17, 80], head: [9, 18], x: 'nearest', y: 'nearest'}),
    reveal('center forward oversized range falls back to end', {anchor: [5, 2], head: [15, 60], x: 'center', y: 'center'}),
    reveal('center backward oversized range falls back to start', {anchor: [15, 60], head: [5, 2], x: 'center', y: 'center'}),
    reveal('scaled and covered centered cursor', {head: [10, 54], assoc: -1, x: 'center', y: 'center',
      scale: [1.25, 1.5], covered: {left: 11, top: 9, right: 8, bottom: 12}}),
    reveal('RTL start aligns right', {rtl: true, head: [12, 70], x: 'start', y: 'end'}),
    reveal('RTL end aligns left', {rtl: true, head: [12, 70], x: 'end', y: 'start'}),
    reveal('LTR clamps both margin signs', {head: [12, 33], x: 'start', y: 'end', xMargin: -500, yMargin: 700}),
    reveal('disabled horizontal scrolling', {hiddenX: true, head: [12, 70], x: 'start', y: 'end'}),
  ];
  globalThis.nativeSupport = cases.map(value => JSON.stringify(value));
} catch (error) {
  console.error(error);
  globalThis.nativeSupport = [JSON.stringify({error: error.stack})];
}
