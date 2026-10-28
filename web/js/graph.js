// Dependency graph: a small layered (Sugiyama-style) layout rendered as SVG.
// Edges point from importer to imported module, so columns read left→right
// from entry points towards the foundations they depend on.
import { h, s } from './dom.js';

const MAX_NODES = 60;
const COL_W = 220;
const ROW_H = 34;
const NODE_W = 184;
const NODE_H = 24;
const PAD = 16;

function basename(p) { const i = p.lastIndexOf('/'); return i >= 0 ? p.slice(i + 1) : p; }
function trunc(t, n) { return t.length > n ? `${t.slice(0, n - 1)}…` : t; }

/** Choose the most connected nodes, always keeping entry points and mother candidates. */
export function pickNodes(graph, mother) {
  const nodes = (graph.nodes || []).slice();
  const score = (n) => (n.fan_in || 0) * 2 + (n.fan_out || 0) + (n.entry ? 6 : 0) + (mother.has(n.path) ? 6 : 0) - (n.test ? 3 : 0);
  nodes.sort((a, b) => score(b) - score(a) || a.path.localeCompare(b.path));
  const kept = nodes.slice(0, MAX_NODES);
  const ids = new Set(kept.map((n) => n.id));
  const edges = (graph.edges || []).filter((e) => ids.has(e.from) && ids.has(e.to) && e.from !== e.to);
  return { nodes: kept, edges, hidden: Math.max(0, nodes.length - kept.length) };
}

function layout(nodes, edges) {
  const out = new Map(nodes.map((n) => [n.id, []]));
  const incoming = new Map(nodes.map((n) => [n.id, 0]));
  edges.forEach((e) => { out.get(e.from).push(e.to); incoming.set(e.to, incoming.get(e.to) + 1); });

  // Break cycles: DFS from roots (entry points first), drop back edges.
  const state = new Map();
  const back = new Set();
  const order = [];
  const roots = nodes.filter((n) => n.entry || incoming.get(n.id) === 0).map((n) => n.id);
  const visit = (id) => {
    state.set(id, 1);
    for (const to of out.get(id)) {
      if (state.get(to) === 1) back.add(`${id}→${to}`);
      else if (!state.get(to)) visit(to);
    }
    state.set(id, 2);
    order.push(id);
  };
  roots.forEach((id) => { if (!state.get(id)) visit(id); });
  nodes.forEach((n) => { if (!state.get(n.id)) visit(n.id); });

  // Longest-path layering over the DAG (reverse post-order = topological).
  const layer = new Map(nodes.map((n) => [n.id, 0]));
  for (const id of order.slice().reverse()) {
    for (const to of out.get(id)) {
      if (back.has(`${id}→${to}`)) continue;
      layer.set(to, Math.max(layer.get(to), layer.get(id) + 1));
    }
  }

  const columns = [];
  nodes.forEach((n) => {
    const l = layer.get(n.id);
    (columns[l] = columns[l] || []).push(n.id);
  });
  for (let i = 0; i < columns.length; i++) if (!columns[i]) columns[i] = [];
  columns.forEach((col) => col.sort());

  // Two barycentre sweeps to reduce crossings.
  const preds = new Map(nodes.map((n) => [n.id, []]));
  edges.forEach((e) => { if (!back.has(`${e.from}→${e.to}`)) preds.get(e.to).push(e.from); });
  for (let pass = 0; pass < 2; pass++) {
    for (let c = 1; c < columns.length; c++) {
      const pos = new Map(columns[c - 1].map((id, i) => [id, i]));
      const bary = (id) => {
        const p = preds.get(id).filter((x) => pos.has(x));
        return p.length ? p.reduce((a, x) => a + pos.get(x), 0) / p.length : Infinity;
      };
      columns[c].sort((a, b) => bary(a) - bary(b) || a.localeCompare(b));
    }
  }

  const pos = new Map();
  const maxRows = Math.max(1, ...columns.map((c) => c.length));
  columns.forEach((col, c) => {
    const offset = ((maxRows - col.length) * ROW_H) / 2;
    col.forEach((id, r) => pos.set(id, { x: PAD + c * COL_W, y: PAD + offset + r * ROW_H }));
  });
  return { pos, width: PAD * 2 + Math.max(1, columns.length) * COL_W - (COL_W - NODE_W), height: PAD * 2 + maxRows * ROW_H };
}

/**
 * Render the graph. Returns { el, select(path) }.
 * opts: { mother: Set<path>, onSelect(path) }
 */
export function renderGraph(graph, opts) {
  const mother = opts.mother || new Set();
  const { nodes, edges, hidden } = pickNodes(graph, mother);
  if (!nodes.length) return { el: h('p', { class: 'muted' }, 'No module relationships were detected.'), select() {} };

  const { pos, width, height } = layout(nodes, edges);
  const svg = s('svg', {
    class: 'graph-svg', viewBox: `0 0 ${width} ${height}`, width, height,
    role: 'group', 'aria-label': `Dependency graph with ${nodes.length} modules and ${edges.length} import relationships`,
  });
  svg.append(s('defs', null,
    s('marker', { id: 'arrow', viewBox: '0 0 10 10', refX: 9, refY: 5, markerWidth: 7, markerHeight: 7, orient: 'auto-start-reverse' },
      s('path', { d: 'M0,0 L10,5 L0,10 z', class: 'graph-arrow' }))));

  const edgeLayer = s('g', { class: 'graph-edges' });
  const edgeEls = edges.map((e) => {
    const a = pos.get(e.from); const b = pos.get(e.to);
    const x1 = a.x + NODE_W; const y1 = a.y + NODE_H / 2;
    const x2 = b.x; const y2 = b.y + NODE_H / 2;
    let d;
    if (x2 > x1) {
      const mx = (x1 + x2) / 2;
      d = `M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2 - 2},${y2}`;
    } else {
      // Back/same-column edge: loop around below.
      const dip = Math.max(y1, y2) + ROW_H;
      d = `M${x1},${y1} C${x1 + 40},${dip} ${x2 - 40},${dip} ${x2 - 2},${y2}`;
    }
    const p = s('path', { d, class: 'graph-edge', 'marker-end': 'url(#arrow)', dataset: { from: e.from, to: e.to } });
    edgeLayer.append(p);
    return p;
  });
  svg.append(edgeLayer);

  const nodeEls = new Map();
  const nodeLayer = s('g', { class: 'graph-nodes' });
  nodes.forEach((n) => {
    const p = pos.get(n.id);
    const label = basename(n.path);
    const g = s('g', {
      class: ['graph-node', n.entry && 'entry', mother.has(n.path) && 'mother', n.test && 'test'],
      transform: `translate(${p.x},${p.y})`, tabindex: 0, role: 'button',
      'aria-label': `${n.path}${n.entry ? ', entry point' : ''}${mother.has(n.path) ? ', central component candidate' : ''}. Used by ${n.fan_in || 0}, imports ${n.fan_out || 0}.`,
    },
    s('title', null, n.path),
    s('rect', { width: NODE_W, height: NODE_H, rx: 5 }),
    n.entry ? s('rect', { class: 'entry-bar', width: 4, height: NODE_H, rx: 2 }) : null,
    s('text', { x: n.entry ? 12 : 9, y: NODE_H / 2 + 4 }, trunc(label, 22)),
    mother.has(n.path) ? s('path', { class: 'mother-mark', d: `M${NODE_W - 14},${NODE_H / 2} l5,-5 l5,5 l-5,5 z` }) : null);
    const pick = () => opts.onSelect && opts.onSelect(n.path);
    g.addEventListener('click', pick);
    g.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); pick(); } });
    nodeLayer.append(g);
    nodeEls.set(n.path, g);
  });
  svg.append(nodeLayer);

  const select = (path, scroll = true) => {
    nodeEls.forEach((g, p) => g.classList.toggle('selected', p === path));
    const related = new Set([path]);
    edgeEls.forEach((el) => {
      const on = el.dataset.from === path || el.dataset.to === path;
      el.classList.toggle('active', on);
      if (on) { related.add(el.dataset.from); related.add(el.dataset.to); }
    });
    svg.classList.toggle('has-selection', !!path && nodeEls.has(path));
    nodeEls.forEach((g, p) => g.classList.toggle('related', related.has(p)));
    const sel = nodeEls.get(path);
    if (sel && scroll) {
      const wrap = svg.parentElement;
      if (wrap) {
        const p = pos.get(path) || { x: 0, y: 0 };
        wrap.scrollTo({ left: Math.max(0, p.x - wrap.clientWidth / 2 + NODE_W / 2), top: Math.max(0, p.y - wrap.clientHeight / 2), behavior: window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
      }
    }
  };

  const legend = h('ul', { class: 'graph-legend', 'aria-label': 'Legend' },
    h('li', null, h('span', { class: 'lg lg-entry' }), 'Entry point'),
    h('li', null, h('span', { class: 'lg lg-mother' }), 'Central component candidate'),
    h('li', null, h('span', { class: 'lg lg-test' }), 'Test'),
    h('li', null, h('span', { class: 'lg lg-edge' }), 'imports →'),
    hidden ? h('li', { class: 'muted' }, `${hidden} less-connected modules hidden`) : null);

  const el = h('div', { class: 'graph' }, h('div', { class: 'graph-scroll' }, svg), legend);
  return { el, select };
}
