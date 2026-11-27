// Run with the integrity-checked upstream browser import map.
import {EditorSelection, Prec} from '@codemirror/state';
import {EditorView} from '@codemirror/view';

const doc = Array.from({length: 50}, (_, i) => `${i} ${'text '.repeat(32)}`).join('\n');
const rect = ({left, top, right, bottom}) => ({left, top, right, bottom});
const offset = ({scrollLeft: x, scrollTop: y}) => ({x, y});
const targetData = ({range, x, y, xMargin, yMargin}) =>
  ({anchor: range.anchor, head: range.head, x, y, xMargin, yMargin});

const handlers = [
  {name: 'default-false', action: 'false'},
  {name: 'low-true', action: 'true', precedence: 'low'},
  {name: 'high-throw', action: 'throw', precedence: 'high'},
  {name: 'highest-false', action: 'false', precedence: 'highest'},
  {name: 'default-true', action: 'true'},
];
const fallingThrough = handlers.filter(({action}) => action !== 'true');
const providers = [
  {name: 'partial', edges: {left: 54, top: 67, right: 258}},
  {name: 'decline', edges: null, precedence: 'high'},
  {name: 'stronger', edges: {left: 65, top: 65, right: 245, bottom: 189}},
  {name: 'outside', edges: {left: 0, bottom: 900}},
  {name: 'legacy', insets: {left: -15, top: 20, right: 5, bottom: 28}},
];

function run(name, handlers, providers, {snapshot = false, focused = false} = {}) {
  const calls = [], measurements = [];
  let active = false, input, legacyBounds;
  const register = (spec, extension) => spec.precedence ? Prec[spec.precedence](extension) : extension;
  const extensions = [EditorView.exceptionSink.of(error => calls.push(`error:${error.message}`))];
  for (const spec of handlers) {
    extensions.push(register(spec, EditorView.scrollHandler.of((view, range, target) => {
      if (!active || range !== target.range) throw Error('invalid handler call');
      calls.push(spec.name);
      const received = {
        docLength: view.state.doc.length, viewport: [view.viewport.from, view.viewport.to],
        focused: view.hasFocus, target: targetData(target),
      };
      if (input && JSON.stringify(input) !== JSON.stringify(received)) throw Error('handler input changed');
      input = received;
      if (spec.action === 'throw') throw Error('handler failed');
      return spec.action === 'true';
    })));
  }
  for (const spec of providers) {
    const facet = spec.insets ? EditorView.scrollMargins : EditorView.coveredMargins;
    extensions.push(register(spec, facet.of(view => {
      if (!active) throw Error('invalid provider call');
      calls.push(spec.name);
      if (!spec.insets) return spec.edges && {elt: view.scrollDOM, ...spec.edges};
      const bounds = view.viewState.scrollParent.getBoundingClientRect();
      legacyBounds = {
        left: Math.max(0, bounds.left), top: Math.max(0, bounds.top),
        right: Math.min(innerWidth, bounds.right), bottom: Math.min(innerHeight, bounds.bottom),
      };
      return spec.insets;
    })));
  }
  const parent = document.body.appendChild(document.createElement('div'));
  parent.style.cssText = 'width:270px;height:180px;overflow:auto;position:relative';
  parent.appendChild(document.createElement('div')).style.height = '1000px';
  parent.scrollTop = 90;
  const view = new EditorView({parent, doc, extensions});
  const scroll = view.scrollDOM, scaleX = 1.25, scaleY = 1.5;
  scroll.style.cssText = 'position:fixed;left:40px;top:50px;width:180px;height:100px;overflow:auto;'
    + 'border-right:8px solid;border-bottom:6px solid;'
    + `transform-origin:top left;transform:scale(${scaleX},${scaleY});`;
  view.contentDOM.style.cssText = 'padding-top:4px;font:16px/20px monospace;';
  if (focused) view.focus();
  scroll.scrollLeft = 75;
  scroll.scrollTop = 119;
  view.measure();
  let effect, snapshotGeometry;
  if (snapshot) {
    const state = view.viewState;
    const block = ({from, top}) => ({from, top});
    snapshotGeometry = {
      offset: offset(scroll), paddingTop: state.paddingTop, viewportFrom: state.viewport.from,
      firstViewportBlock: block(state.viewportLines[0]),
    };
    const lineAtHeight = state.lineBlockAtHeight;
    state.lineBlockAtHeight = function(height) {
      const result = lineAtHeight.call(this, height);
      snapshotGeometry.probe = {height, block: block(result)};
      return result;
    };
    effect = view.scrollSnapshot();
    state.lineBlockAtHeight = lineAtHeight;
    scroll.scrollLeft = 0;
    scroll.scrollTop = 0;
    view.measure();
  } else {
    const line = view.state.doc.line(11);
    const range = EditorSelection.range(line.from + 56, line.from + 49);
    effect = EditorView.scrollIntoView(range, {x: 'center', y: 'center', xMargin: 7, yMargin: 9});
  }
  const {left, top} = scroll.getBoundingClientRect();
  const clip = {left, top, right: left + scroll.clientWidth * scaleX, bottom: top + scroll.clientHeight * scaleY};
  const scroller = {
    clip, scaleX, scaleY, offsetWidth: scroll.offsetWidth, offsetHeight: scroll.offsetHeight,
    scrollX: scroll.scrollWidth > scroll.clientWidth, scrollY: scroll.scrollHeight > scroll.clientHeight,
  };
  const pinnedCoords = view.docView.coordsAt;
  view.docView.coordsAt = function(pos, side) {
    const result = pinnedCoords.call(this, pos, side);
    if (active) {
      calls.push('coords');
      measurements.push({pos, side, rect: result ? rect(result) : null});
    }
    return result;
  };
  let restored, before, actual;
  const assigned = {};
  const pinnedScroll = view.docView.scrollIntoView;
  view.docView.scrollIntoView = function(target) {
    if (before || target.isSnapshot !== snapshot) throw Error('unexpected scroll request');
    before = offset(scroll);
    if (snapshot) restored = view.viewState.lineBlockAt(target.range.head).top;
    for (const [axis, property] of [['x', 'scrollLeft'], ['y', 'scrollTop']]) {
      const {get, set} = Object.getOwnPropertyDescriptor(Element.prototype, property);
      Object.defineProperty(scroll, property, {
        configurable: true,
        get() { return get.call(this); },
        set(value) { assigned[axis] = value; set.call(this, value); },
      });
    }
    active = true;
    try {
      return pinnedScroll.call(this, target);
    } finally {
      active = false;
      delete scroll.scrollLeft;
      delete scroll.scrollTop;
      actual = offset(scroll);
    }
  };
  const parentBefore = parent.scrollTop;
  view.dispatch({effects: effect});
  view.measure();
  if (!before || parent.scrollTop !== parentBefore) throw Error('expected only the fixed scroller to move');
  view.destroy();
  parent.remove();
  return {
    name, snapshot, handlers, providers, input, calls, measurements, scroller, legacyBounds,
    target: targetData(effect.value), before, assigned, actual, restored, snapshotGeometry,
  };
}

try {
  globalThis.nativeSupport = [
    run('first true wins after a thrown handler', handlers, providers, {focused: true}),
    run('all handlers fall through', fallingThrough, []),
    run('covered edges intersect', fallingThrough, providers.slice(0, -1)),
    run('legacy inset reference differs from the client clip', fallingThrough, providers),
    run('snapshot bypasses callbacks', handlers, providers, {snapshot: true}),
  ].map(row => JSON.stringify(row));
} catch (error) {
  globalThis.nativeSupport = [JSON.stringify({error: error.stack})];
}
