// Trust passport: provenance, dependencies and risk signals found by static inspection.
import { h, emptyState, fmtDate, shortSha, sig, copyButton } from '../dom.js';
import { Section } from '../section.js';

const SIGNAL_SECTIONS = [
  ['install_scripts', 'Install and startup scripts', 'Code that runs when dependencies are installed or the project starts.'],
  ['network_access', 'Network access', 'Places where the code may open connections.'],
  ['filesystem_access', 'Filesystem access', 'Reads, writes or deletes outside obvious data files.'],
  ['process_execution', 'Process execution', 'Shelling out or spawning other programs.'],
  ['dynamic_code', 'Dynamic code', 'eval, exec, pickle and similar constructs that can run arbitrary code.'],
];

export class PassportTab {
  constructor(ctx) { this.ctx = ctx; }

  mount() {
    this.el = h('div', { class: 'passport' });
    this.section = new Section(this.el, (d) => this.render(d));
    return this.el;
  }

  update(d) { this.section.update(d, sig([d.passport, d.execution_mode])); }

  render(d) {
    const p = d.passport;
    if (!p) return emptyState('No trust passport yet.', 'The passport is produced by static inspection during the investigation.');
    const prov = p.provenance || {};
    return [
      h('div', { class: 'arch-head' },
        h('h2', null, 'Trust passport'),
        h('p', { class: 'callout callout-info small' },
          h('strong', null, 'Sandbox observation does not prove code is safe. '),
          'These signals come from static inspection of the original source. A finding is not an accusation, and the absence of findings is not a guarantee. Investigate before running anything natively.')),
      h('section', { class: 'card' },
        h('h3', null, 'Provenance'),
        h('dl', { class: 'kv' },
          kv('Source', h('a', { href: prov.source_url, target: '_blank', rel: 'noopener noreferrer', class: 'mono' }, prov.source_url || '—')),
          kv('Original commit', [h('span', { class: 'mono' }, prov.original_commit || '—'), prov.original_commit ? copyButton(prov.original_commit, 'Copy') : null]),
          kv('Cloned', fmtDate(prov.cloned_at, true)),
          kv('Licence', prov.license || 'None declared — public visibility does not by itself grant permission to redistribute'),
          kv('Execution mode', d.execution_mode))),
      (p.secrets || []).length ? h('section', { class: 'card callout-bad-border' },
        h('h3', null, `Possible secrets (${p.secrets.length})`),
        h('p', { class: 'small muted' }, 'Shown only in redacted form. They are excluded from AI prompts and logs. Nothing is rotated or transmitted automatically.'),
        signalTable(p.secrets.map((s) => ({ path: s.path, line: s.line, detail: `${s.kind}: ${s.redacted}` })), this.ctx)) : null,
      h('div', { class: 'passport-grid' }, SIGNAL_SECTIONS.map(([key, title, blurb]) => h('section', { class: 'card' },
        h('h3', null, `${title} (${(p[key] || []).length})`),
        h('p', { class: 'small muted' }, blurb),
        (p[key] || []).length ? signalTable(p[key], this.ctx) : h('p', { class: 'small muted' }, 'None found.')))),
      h('section', { class: 'card' },
        h('h3', null, `Dependencies (${(p.dependencies || []).length})`),
        (p.dependencies || []).length ? h('div', { class: 'table-wrap' }, h('table', { class: 'table' },
          h('thead', null, h('tr', null, ['Ecosystem', 'Package', 'Specifier', 'Declared in'].map((t) => h('th', { scope: 'col' }, t)))),
          h('tbody', null, p.dependencies.map((dep) => h('tr', null,
            h('td', null, dep.ecosystem), h('th', { scope: 'row', class: 'mono' }, dep.name), h('td', { class: 'mono' }, dep.spec || '—'),
            h('td', null, h('button', { class: 'link mono small', type: 'button', onclick: () => this.ctx.openFile(dep.source_path) }, dep.source_path)))))))
          : h('p', { class: 'small muted' }, 'No declared dependencies found.')),
      h('section', { class: 'card' },
        h('h3', null, 'Unresolved risks'),
        (p.unresolved || []).length ? h('ul', null, p.unresolved.map((u) => h('li', null, u))) : h('p', { class: 'small muted' }, 'None recorded.')),
      h('p', { class: 'small muted' }, `Passport for original commit ${shortSha(prov.original_commit)}.`),
    ];
  }
}

function kv(k, v) { return h('div', null, h('dt', null, k), h('dd', null, v)); }

function signalTable(list, ctx) {
  return h('ul', { class: 'signal-rows' }, list.map((s) => h('li', null,
    h('button', { class: 'link mono small', type: 'button', onclick: () => ctx.openFile(s.path, s.line, s.line) }, `${s.path}${s.line ? `:${s.line}` : ''}`),
    h('span', { class: 'small' }, ` ${s.detail || ''}`))));
}
