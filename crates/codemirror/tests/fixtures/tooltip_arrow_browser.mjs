// Capture with capture_browser_fixture.mjs, Chrome, and the SHA-512-pinned archives.
// Entry packages: @codemirror/view,@codemirror/theme-one-dark,@codemirror/lint.
import {Compartment} from '@codemirror/state';
import {EditorView, activateHover, closeHoverTooltips, hoverTooltip, showTooltip, tooltips} from '@codemirror/view';
import {oneDarkTheme} from '@codemirror/theme-one-dark';
import {nextDiagnostic, setDiagnostics} from '@codemirror/lint';

const records = [];
const doc = Array.from({length: 22}, () => '0123456789abcdefghijklmno').join('\n');
const root = document.body.appendChild(document.createElement('div'));
root.style.cssText = 'position:absolute;left:40px;top:60px;width:280px;';
const sizes = EditorView.theme({
  '&': {height: '220px'},
  '.cm-scroller': {overflow: 'auto', fontFamily: 'monospace', fontSize: '16px', lineHeight: '20px'},
  '.cm-tooltip-hover': {width: '96px', height: '32px'},
  '.cm-tooltip.probe-direct': {width: '96px', height: '32px'},
});
const theme = new Compartment();
const direct = new Compartment();
const space = new Compartment();
const rect = {left: 40, top: 60, right: 320, bottom: 280};
let opt = {};
function create(className) {
  const dom = document.createElement('div');
  dom.className = className;
  dom.textContent = 'probe';
  return {dom};
}
const source = hoverTooltip((_v, pos) => ({pos, end: pos + 1, create: () => create('probe'), ...opt}));
const view = new EditorView({doc, parent: root, extensions: [sizes, theme.of(oneDarkTheme), direct.of([]), space.of(tooltips({tooltipSpace: () => rect})), source]});
const keys = (obj, ...names) => Object.fromEntries(names.map(name => [name, obj[name]]));
const bounds = obj => obj == null ? null : [obj.left, obj.top, obj.right, obj.bottom];
const pseudo = (arrow, name) => {
  const css = getComputedStyle(arrow, name);
  const side = part => [css['border' + part + 'Width'], css['border' + part + 'Style'], css['border' + part + 'Color']].join(' ');
  return [css.position, css.content, css.width, css.height, css.top, css.bottom, side('Top'), side('Bottom'), side('Left'), side('Right')];
};
function capture(name, pos, includeCSS = true) {
  view.measure(); view.measure();
  const body = view.dom.querySelector('.cm-tooltip');
  const arrow = body?.querySelector('.cm-tooltip-arrow');
  records.push({name, focused: view.hasFocus, pos, anchor: bounds(view.coordsAtPos(pos)), scroll: bounds(view.scrollDOM.getBoundingClientRect()),
    body: body && bounds(body.getBoundingClientRect()), classes: body?.className, anchoredStyle: body && [body.style.left, body.style.top],
    bodyStyle: includeCSS && body ? keys(getComputedStyle(body), 'position', 'color', 'backgroundColor', 'borderTopWidth', 'borderTopStyle', 'overflow', 'zIndex') : undefined,
    arrow: arrow ? bounds(arrow.getBoundingClientRect()) : null, arrowLeft: arrow?.style.left,
    arrowStyle: includeCSS && arrow ? keys(getComputedStyle(arrow), 'position', 'width', 'height', 'left', 'top', 'bottom', 'overflow', 'zIndex') : undefined,
    before: includeCSS && arrow ? pseudo(arrow, '::before') : undefined, after: includeCSS && arrow ? pseudo(arrow, '::after') : undefined,
  });
}
function open(name, line, col, options, css = true) {
  view.dispatch({effects: closeHoverTooltips});
  opt = options;
  const pos = view.state.doc.line(line).from + col;
  activateHover(view, pos, 1, {tooltip: source});
  capture(name, pos, css);
  return pos;
}
view.focus();
const pos = open('arrow-above-focused', 5, 8, {arrow: true, above: true});
view.contentDOM.blur();
capture('same-arrow-unfocused', pos, false);
view.focus();
open('arrow-below', 5, 8, {arrow: true});
open('arrow-side-flips-near-top', 1, 8, {arrow: true, above: true}, false);
open('arrow-clamped-left', 5, 0, {arrow: true, above: true}, false);
open('arrow-clamped-right', 5, 24, {arrow: true, above: true}, false);
open('arrow-opt-out', 5, 8, {above: true});
view.dispatch({effects: closeHoverTooltips});
view.dispatch({effects: direct.reconfigure(showTooltip.of({pos, arrow: true, above: true, create: () => create('probe-direct')}))});
capture('direct-arrow', pos, false);
view.dispatch({effects: direct.reconfigure([])});
view.dispatch({effects: space.reconfigure(tooltips({tooltipSpace: () => ({...rect, top: 160})}))});
open('anchor-outside-tooltip-space', 3, 8, {arrow: true}, false);
view.dispatch({effects: closeHoverTooltips});
view.dispatch({effects: space.reconfigure(tooltips({tooltipSpace: () => rect}))});
open('arrow-before-scroll', 3, 8, {arrow: true}, false);
view.scrollDOM.scrollTop = 120;
view.scrollDOM.dispatchEvent(new Event('scroll'));
capture('arrow-after-scroll-clipped', view.state.doc.line(3).from + 8, false);
view.dispatch({effects: closeHoverTooltips});
view.scrollDOM.scrollTop = 0;
view.scrollDOM.dispatchEvent(new Event('scroll'));
view.dispatch(setDiagnostics(view.state, [{from: pos, to: pos + 1, severity: 'error', message: 'lint probe'}]));
if (!nextDiagnostic(view)) throw new Error('stock lint F8 did not activate');
capture('stock-lint-F8', pos);
view.dispatch({effects: closeHoverTooltips});
view.dispatch({effects: theme.reconfigure([])});
open('light-base-arrow', 5, 8, {arrow: true, above: true});
view.destroy();
globalThis.nativeSupport = records.map(record => JSON.stringify(record));
