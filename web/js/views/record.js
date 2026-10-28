// Inheritance Record and lineage: original → successor, export, full diff.
import { h, replace, emptyState, fmtDate, fmtNum, shortSha, sig, spinner, errorBox, diffView, copyButton } from '../dom.js';
import { api } from '../api.js';
import { Section } from '../section.js';

export class RecordTab {
  constructor(ctx) { this.ctx = ctx; }

  mount() {
    this.el = h('div', { class: 'record' });
    this.section = new Section(this.el, (d) => this.render(d));
    this.diffEl = h('div', { class: 'diff-slot' });
    return h('div', null, this.el, this.diffEl);
  }

  update(d) { this.section.update(d, sig([d.lineage, d.successor, d.original, d.counts, (d.issues || []).length, (d.repairs || []).length, (d.verifications || []).length, !!d.report, d.license])); }

  render(d) {
    const lineage = d.lineage || [];
    const r = d.report || {};
    const claims = [r.purpose, ...(r.capabilities || []), ...(r.data_flows || []), ...(r.external_services || [])].filter(Boolean).length;
    const diffBtn = h('button', { class: 'btn', type: 'button' }, 'View diff vs original');
    diffBtn.addEventListener('click', () => this.loadDiff(diffBtn));

    return [
      h('div', { class: 'arch-head' },
        h('h2', null, 'Inheritance Record'),
        h('p', { class: 'small muted' }, 'A portable, versioned JSON record of where this software came from, what was learned, what changed and how it was verified. Lineage does not imply endorsement by the original creator.')),
      h('div', { class: 'record-grid' },
        h('section', { class: 'card' },
          h('h3', null, 'Lineage'),
          lineage.length ? h('ol', { class: 'lineage' }, lineage.map((e) => h('li', { class: `lineage-item lineage-${e.kind}` },
            h('span', { class: 'lineage-dot', 'aria-hidden': 'true' }),
            h('div', null,
              h('p', { class: 'lineage-label' },
                h('span', { class: `lineage-kind kind-${e.kind}` }, e.kind === 'original' ? 'Original' : 'Successor'),
                ' ', e.label),
              h('p', { class: 'small muted' },
                h('span', { class: 'mono' }, shortSha(e.sha)), e.sha ? copyButton(e.sha, 'Copy') : null,
                ` · ${e.author || 'unknown author'} · ${fmtDate(e.date, true)}`)))))
            : emptyState('No lineage yet.', 'The original commit appears here once the repository is cloned.')),
        h('section', { class: 'card' },
          h('h3', null, 'What the record contains'),
          h('dl', { class: 'kv' },
            kv('Original', `${d.full_name} @ ${shortSha(d.original_commit)}`),
            kv('Original licence', d.license || 'None declared'),
            kv('Successor', `${(d.successor || {}).name || '—'} on branch ${(d.successor || {}).branch || '—'}`),
            kv('Archaeology', `${fmtNum(claims)} claims, ${fmtNum((r.components || []).length)} components, ${fmtNum((r.unknowns || []).length)} unknowns`),
            kv('Restoration', `${fmtNum((d.issues || []).length)} issues, ${fmtNum((d.repairs || []).length)} repair attempts`),
            kv('Verification', `${fmtNum((d.verifications || []).length)} recorded runs`),
            kv('Provenance', `${fmtNum(d.counts?.agent_calls ?? 0)} agent calls referenced by id`)),
          h('p', { class: 'small muted' }, 'Evidence is stored as file paths, line ranges and commit SHAs rather than copies of source code.'),
          h('div', { class: 'card-actions' },
            h('a', { class: 'btn btn-primary', href: api.recordUrl(this.ctx.id), download: `${d.name || 'inheritance'}-record.json` }, 'Download record (JSON)'),
            diffBtn))),
    ];
  }

  async loadDiff(btn) {
    btn.disabled = true;
    replace(this.diffEl, h('div', { class: 'loading-row' }, spinner('Loading diff'), ' Loading diff…'));
    try {
      const r = await api.diff(this.ctx.id);
      replace(this.diffEl, h('section', { class: 'card' },
        h('h3', null, 'Successor vs original'),
        h('p', { class: 'small muted' }, h('span', { class: 'mono' }, shortSha(r.base)), ' → ', h('span', { class: 'mono' }, shortSha(r.head))),
        diffView(r.diff)));
    } catch (e) {
      replace(this.diffEl, errorBox(e.message));
    } finally {
      btn.disabled = false;
    }
  }
}

function kv(k, v) { return h('div', null, h('dt', null, k), h('dd', null, v)); }
