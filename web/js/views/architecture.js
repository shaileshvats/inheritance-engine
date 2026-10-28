// Architecture: repository tree, dependency graph, selection panel, inventory facts.
import { h, replace, pill, emptyState, fmtNum, fmtBytes, fmtDate, shortSha, sig, storage, store, details } from '../dom.js';
import { renderGraph, pickNodes } from '../graph.js';
import { Section } from '../section.js';

export class ArchitectureTab {
  constructor(ctx) {
    this.ctx = ctx;
    this.selected = null;
    this.showTable = storage('ie-graph-table', false);
    this.graph = null;
  }

  mount() {
    this.el = h('div', { class: 'architecture' });
    this.section = new Section(this.el, (d) => this.render(d));
    return this.el;
  }

  update(d) {
    this.d = d;
    if (this.section.update(d, sig([d.inventory, d.report && d.report.components, d.report && d.report.purpose, this.showTable]))) {
      if (this.selected) this.select(this.selected, { scroll: false });
    }
  }

  focus(f) {
    if (f && f.path) { this.pendingPath = f.path; if (this.d) this.select(f.path); }
  }

  render(d) {
    const inv = d.inventory;
    if (!inv) {
      return emptyState('No inventory yet.', d.active_job ? 'The engine is cataloguing files, imports and history…' : 'Run an investigation to build the inventory.');
    }
    this.mother = new Set((inv.mother_candidates || []).slice(0, 6).map((m) => m.path));
    this.panelEl = h('aside', { class: 'selection-panel card', 'aria-live': 'polite', 'aria-label': 'Selected component' });
    replace(this.panelEl, h('p', { class: 'muted' }, 'Select a file in the tree or a node in the graph to see its role, callers, dependencies and evidence.'));

    const graphBox = h('div', { class: 'graph-box card' });
    const toggle = h('button', { class: 'btn btn-ghost btn-xs', type: 'button', 'aria-pressed': String(this.showTable) },
      this.showTable ? 'Show graph' : 'Show as table');
    toggle.addEventListener('click', () => { this.showTable = !this.showTable; store('ie-graph-table', this.showTable); this.section.invalidate(); this.update(d); });

    if (this.showTable) {
      graphBox.append(this.graphTable(inv));
      this.graph = null;
    } else {
      this.graph = renderGraph(inv.graph || {}, { mother: this.mother, onSelect: (p) => this.select(p) });
      graphBox.append(this.graph.el);
    }

    const out = [
      h('div', { class: 'arch-head' },
        h('h2', null, 'Architecture'),
        h('p', { class: 'small muted' }, 'Derived deterministically from the source: imports, entry points, configuration and git history. No AI is involved in this map.')),
      h('div', { class: 'arch-grid' },
        h('section', { class: 'tree-box card', 'aria-labelledby': 'tree-title' },
          h('h3', { id: 'tree-title' }, 'Repository'),
          h('p', { class: 'small muted' }, `${fmtNum(inv.files_total)} files · ${fmtBytes(inv.bytes_total)}`),
          h('div', { class: 'tree' }, this.tree(inv.tree, 0))),
        h('section', { class: 'graph-col', 'aria-labelledby': 'graph-title' },
          h('div', { class: 'graph-head' }, h('h3', { id: 'graph-title' }, 'Dependency graph'), toggle),
          graphBox),
        this.panelEl),
      h('div', { class: 'inv-grid' },
        this.motherCard(inv),
        this.languagesCard(inv),
        this.structureCard(inv),
        this.gitCard(inv)),
    ];
    if (this.pendingPath) { const p = this.pendingPath; this.pendingPath = null; requestAnimationFrame(() => this.select(p)); }
    return out;
  }

  tree(node, depth) {
    if (!node) return null;
    if (node.kind === 'file') {
      return h('button', {
        class: ['tree-file', this.mother.has(node.path) && 'is-mother', this.selected === node.path && 'is-selected'],
        type: 'button', dataset: { path: node.path }, onclick: () => this.select(node.path),
      }, h('span', { class: 'tree-icon', 'aria-hidden': 'true' }, '▫'), node.name,
      this.mother.has(node.path) ? h('span', { class: 'sr-only' }, ' (central component candidate)') : null);
    }
    const children = (node.children || []).slice().sort((a, b) => (a.kind === b.kind ? a.name.localeCompare(b.name) : a.kind === 'dir' ? -1 : 1));
    const list = h('ul', { class: 'tree-list' }, children.map((c) => h('li', null, this.tree(c, depth + 1))));
    if (depth === 0) return list;
    return details(`tree:${node.path}`, this.ctx.openSet,
      h('span', { class: 'tree-dir' }, h('span', { class: 'tree-icon', 'aria-hidden': 'true' }, '▸'), node.name),
      list, { openByDefault: depth <= 1 });
  }

  select(path, { scroll = true } = {}) {
    this.selected = path;
    if (!this.d || !this.d.inventory || !this.panelEl) return;
    if (this.graph) this.graph.select(path, scroll);
    this.el.querySelectorAll('.tree-file').forEach((b) => b.classList.toggle('is-selected', b.dataset.path === path));
    replace(this.panelEl, this.panel(path));
  }

  panel(path) {
    const d = this.d; const inv = d.inventory; const r = d.report || {};
    const g = inv.graph || { nodes: [], edges: [] };
    const node = (g.nodes || []).find((n) => n.path === path);
    const callers = (g.edges || []).filter((e) => e.to === path).map((e) => e.from);
    const deps = (g.edges || []).filter((e) => e.from === path).map((e) => e.to);
    const mother = (inv.mother_candidates || []).find((m) => m.path === path);
    const entry = (inv.entry_points || []).filter((e) => e.path === path);
    const comps = (r.components || []).filter((c) => c.path === path);
    const claims = [r.purpose, ...(r.capabilities || []), ...(r.data_flows || []), ...(r.external_services || [])]
      .filter((c) => c && (c.evidence || []).some((e) => e.path === path));
    const decay = (inv.decay || []).filter((x) => x.path === path);
    const issues = (d.issues || []).filter((i) => (i.evidence || []).some((e) => e.path === path));
    const pathList = (list) => list.length
      ? h('ul', { class: 'path-list' }, list.map((p) => h('li', null, h('button', { class: 'link mono small', type: 'button', onclick: () => this.select(p) }, p))))
      : h('p', { class: 'muted small' }, 'None');

    return [
      h('h3', { class: 'mono panel-path' }, path),
      h('div', { class: 'panel-actions' },
        h('button', { class: 'btn btn-xs', type: 'button', onclick: () => this.ctx.openFile(path) }, 'Open file'),
        node && node.lang ? h('span', { class: 'small muted' }, node.lang) : null),
      h('div', { class: 'panel-tags' },
        entry.length ? pill('verified', 'Entry point') : null,
        mother ? pill('inferred', `Central candidate · ${Math.round(mother.score * 100)}%`) : null,
        node && node.test ? pill('unknown', 'Test') : null),
      node ? h('dl', { class: 'mini-stats' }, h('div', null, h('dt', null, 'Used by'), h('dd', null, fmtNum(node.fan_in))), h('div', null, h('dt', null, 'Imports'), h('dd', null, fmtNum(node.fan_out)))) : null,
      entry.length ? h('ul', { class: 'small' }, entry.map((e) => h('li', null, `${e.kind}${e.detail ? ` (${e.detail})` : ''}`))) : null,
      mother ? h('div', null, h('h4', null, 'Why it may be central'), h('ul', { class: 'small' }, mother.reasons.map((x) => h('li', null, x)))) : null,
      comps.length ? h('div', null, h('h4', null, 'Role (agent team)'), comps.map((c) => h('p', { class: 'small' }, pill(c.status || 'unknown'), ' ', c.role))) : null,
      h('h4', null, `Callers (${callers.length})`), pathList(callers),
      h('h4', null, `Dependencies (${deps.length})`), pathList(deps),
      claims.length ? h('div', null, h('h4', null, 'Findings citing this file'),
        h('ul', { class: 'small' }, claims.map((c) => h('li', null, pill(c.status || 'unknown'), ' ',
          h('button', { class: 'link', type: 'button', onclick: () => this.ctx.goTab('report', { claim: c.id }) }, c.text))))) : null,
      decay.length ? h('div', null, h('h4', null, 'Decay detected'),
        h('ul', { class: 'small' }, decay.map((x) => h('li', null,
          h('button', { class: 'link mono', type: 'button', onclick: () => this.ctx.openFile(path, x.line, x.line) }, `line ${x.line}`),
          ` ${x.pattern} — ${x.detail}`, x.removed_in ? h('span', { class: 'muted' }, ` (removed in ${x.removed_in})`) : null)))) : null,
      issues.length ? h('div', null, h('h4', null, 'Related issues'),
        h('ul', { class: 'small' }, issues.map((i) => h('li', null, pill(i.status), ' ',
          h('button', { class: 'link', type: 'button', onclick: () => this.ctx.goTab('restoration', { issue: i.id }) }, i.title))))) : null,
    ];
  }

  graphTable(inv) {
    const { nodes, edges, hidden } = pickNodes(inv.graph || {}, this.mother);
    if (!nodes.length) return h('p', { class: 'muted' }, 'No module relationships were detected.');
    return h('div', { class: 'table-wrap' },
      h('table', { class: 'table' },
        h('caption', { class: 'sr-only' }, 'Modules and their import relationships'),
        h('thead', null, h('tr', null, ['Module', 'Kind', 'Used by', 'Imports', 'Depends on'].map((t) => h('th', { scope: 'col' }, t)))),
        h('tbody', null, nodes.map((n) => h('tr', null,
          h('th', { scope: 'row' }, h('button', { class: 'link mono small', type: 'button', onclick: () => this.select(n.path) }, n.path)),
          h('td', null, [n.entry && 'entry', this.mother.has(n.path) && 'central', n.test && 'test'].filter(Boolean).join(', ') || '—'),
          h('td', null, fmtNum(n.fan_in)),
          h('td', null, fmtNum(n.fan_out)),
          h('td', { class: 'mono small' }, edges.filter((e) => e.from === n.id).map((e) => e.to).join(', ') || '—'))))),
      hidden ? h('p', { class: 'small muted' }, `${hidden} less-connected modules not shown.`) : null);
  }

  motherCard(inv) {
    const list = inv.mother_candidates || [];
    return h('section', { class: 'card' },
      h('h3', null, 'Central component candidates'),
      h('p', { class: 'small muted' }, 'Ranked by entry points, import fan-in, git churn and documentation mentions — not by file size.'),
      list.length ? h('ol', { class: 'mother-list' }, list.map((m) => h('li', null,
        h('div', { class: 'mother-row' },
          h('button', { class: 'link mono', type: 'button', onclick: () => this.select(m.path) }, m.path),
          h('span', { class: 'score-bar', role: 'img', 'aria-label': `score ${Math.round(m.score * 100)} percent` },
            h('span', { style: { width: `${Math.max(4, Math.min(100, m.score * 100))}%` } }))),
        h('p', { class: 'small muted' }, (m.reasons || []).join(' · ')))))
        : h('p', { class: 'muted' }, 'None identified.'));
  }

  languagesCard(inv) {
    const langs = inv.languages || [];
    const total = langs.reduce((a, l) => a + (l.lines || 0), 0) || 1;
    return h('section', { class: 'card' },
      h('h3', null, 'Languages'),
      langs.length ? [
        h('div', { class: 'lang-bar', role: 'img', 'aria-label': langs.map((l) => `${l.language} ${Math.round((l.lines / total) * 100)}%`).join(', ') },
          langs.map((l, i) => h('span', { class: `lang-seg lang-${i % 6}`, style: { width: `${(l.lines / total) * 100}%` }, title: `${l.language}: ${fmtNum(l.lines)} lines` }))),
        h('ul', { class: 'lang-list' }, langs.map((l, i) => h('li', null, h('span', { class: `lang-dot lang-${i % 6}`, 'aria-hidden': 'true' }),
          `${l.language} `, h('span', { class: 'muted small' }, `${fmtNum(l.files)} files · ${fmtNum(l.lines)} lines`)))),
      ] : h('p', { class: 'muted' }, 'No source files recognised.'));
  }

  structureCard(inv) {
    const tests = inv.tests || {};
    return h('section', { class: 'card' },
      h('h3', null, 'Structure'),
      h('h4', null, 'Manifests'),
      (inv.manifests || []).length ? h('ul', { class: 'small' }, inv.manifests.map((m) => h('li', null, h('span', { class: 'mono' }, m.path), h('span', { class: 'muted' }, ` · ${m.kind}`)))) : h('p', { class: 'muted small' }, 'None found.'),
      h('h4', null, 'Entry points'),
      (inv.entry_points || []).length ? h('ul', { class: 'small' }, inv.entry_points.map((e) => h('li', null,
        h('button', { class: 'link mono', type: 'button', onclick: () => this.select(e.path) }, e.path),
        h('span', { class: 'muted' }, ` · ${e.kind}${e.detail ? ` (${e.detail})` : ''}`)))) : h('p', { class: 'muted small' }, 'None detected.'),
      h('h4', null, 'Tests'),
      h('p', { class: 'small' }, `Framework: ${tests.framework || 'none'} · ${fmtNum((tests.files || []).length)} test files`),
      (tests.files || []).length ? details('test-files', this.ctx.openSet, 'Show test files',
        h('ul', { class: 'small mono' }, tests.files.map((f) => h('li', null, f)))) : null);
  }

  gitCard(inv) {
    const g = inv.git;
    if (!g) return h('section', { class: 'card' }, h('h3', null, 'History'), h('p', { class: 'muted' }, 'No git history available.'));
    return h('section', { class: 'card' },
      h('h3', null, 'History'),
      h('dl', { class: 'mini-stats' },
        h('div', null, h('dt', null, 'Commits'), h('dd', null, fmtNum(g.commits))),
        h('div', null, h('dt', null, 'First'), h('dd', null, fmtDate(g.first_commit_at))),
        h('div', null, h('dt', null, 'Last'), h('dd', null, fmtDate(g.last_commit_at)))),
      (g.authors || []).length ? [h('h4', null, 'Authors'),
        h('ul', { class: 'small' }, g.authors.slice(0, 8).map((a) => h('li', null, a.name, h('span', { class: 'muted' }, ` · ${fmtNum(a.commits)} commits`))))] : null,
      (g.recent || []).length ? [h('h4', null, 'Most recent commits'),
        h('ul', { class: 'commit-list small' }, g.recent.slice(0, 8).map((c) => h('li', null,
          h('span', { class: 'mono' }, shortSha(c.sha)), ' ', c.subject,
          h('span', { class: 'muted' }, ` — ${c.author}, ${fmtDate(c.date)}`))))] : null);
  }
}
