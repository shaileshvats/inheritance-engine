// Agent trace: every real model invocation, nested by delegation.
import { h, replace, pill, emptyState, fmtDate, fmtDuration, fmtNum, spinner, errorBox, titleCase, storage, store, sig } from '../dom.js';
import { api } from '../api.js';

const ROLES = {
  lead: { label: 'Lead Engineer', blurb: 'Plans, delegates, resolves conflicts and talks to you.' },
  historian: { label: 'Historian / Architect', blurb: 'Reconstructs purpose, architecture and history.' },
  restorer: { label: 'Restorer', blurb: 'Diagnoses problems and writes repairs.' },
  reviewer: { label: 'Reviewer / Tester', blurb: 'Challenges claims and reviews every repair.' },
  hunter: { label: 'Hunter', blurb: 'Turns problems into searches and explains candidates.' },
  tester: { label: 'Tester', blurb: 'Writes behaviour probes comparing the successor with the original.' },
  triager: { label: 'Issue Triager', blurb: 'Mastra workflow: classifies upstream issues against the successor.' },
};

export class AgentsTab {
  constructor(ctx) {
    this.ctx = ctx;
    this.dev = storage('ie-dev-mode', false);
    this.calls = null;
    this.callsSig = null;
    this.expanded = new Set();
    this.cache = new Map();
  }

  mount() {
    const toggle = h('input', { type: 'checkbox', checked: this.dev, id: 'dev-mode' });
    toggle.addEventListener('change', () => { this.dev = toggle.checked; store('ie-dev-mode', this.dev); this.render(); });
    this.listEl = h('div', { class: 'trace' }, spinner('Loading agent trace'));
    this.teamEl = h('div', { class: 'team' });
    return h('div', { class: 'agents' },
      h('div', { class: 'arch-head agents-head' },
        h('div', null,
          h('h2', null, 'Agent trace'),
          h('p', { class: 'small muted' }, 'Each row is a real, recorded call to the local open-weight model. Specialists receive only the files listed for their task. Secrets are redacted before anything is stored.')),
        h('label', { class: 'switch' }, toggle, h('span', null, 'Developer mode'))),
      this.teamEl,
      this.listEl);
  }

  update(d) {
    const n = d.counts && d.counts.agent_calls;
    const key = sig([n, d.active_job && d.active_job.step]);
    if (key === this.callsSig) return;
    this.callsSig = key;
    this.load();
  }

  focus(f) {
    if (f && f.call) { this.expanded.add(f.call); this.focusCall = f.call; if (this.calls) this.render(); }
  }

  async load() {
    try {
      this.calls = await api.calls(this.ctx.id);
      this.render();
    } catch (e) {
      replace(this.listEl, errorBox(e.message));
    }
  }

  render() {
    if (!this.calls) return;
    const calls = this.calls;
    // Team summary
    const byRole = new Map();
    calls.forEach((c) => {
      const r = byRole.get(c.agent) || { n: 0, ms: 0, errors: 0 };
      r.n++; r.ms += c.duration_ms || 0; if (!c.ok) r.errors++;
      byRole.set(c.agent, r);
    });
    replace(this.teamEl, h('ul', { class: 'team-grid', 'aria-label': 'Agent team' },
      Object.entries(ROLES).filter(([k]) => k !== 'hunter' || byRole.has(k)).map(([k, role]) => {
        const r = byRole.get(k) || { n: 0, ms: 0, errors: 0 };
        return h('li', { class: `team-card role-${k}` },
          h('span', { class: `role-badge role-${k}` }, role.label),
          h('p', { class: 'small muted' }, role.blurb),
          h('p', { class: 'small' }, `${fmtNum(r.n)} call${r.n === 1 ? '' : 's'} · ${fmtDuration(r.ms)}`, r.errors ? h('span', { class: 'tone-text-bad' }, ` · ${r.errors} failed`) : null));
      })));

    if (!calls.length) {
      replace(this.listEl, emptyState('No agent calls yet.', 'Agent calls appear here as soon as the Lead Engineer starts the investigation.'));
      return;
    }
    // Build parent → children
    const ids = new Set(calls.map((c) => c.id));
    const kids = new Map();
    const roots = [];
    calls.slice().sort((a, b) => String(a.started_at).localeCompare(String(b.started_at))).forEach((c) => {
      if (c.parent_id && ids.has(c.parent_id)) {
        if (!kids.has(c.parent_id)) kids.set(c.parent_id, []);
        kids.get(c.parent_id).push(c);
      } else roots.push(c);
    });
    const renderNode = (c) => h('li', null, this.row(c), kids.has(c.id) ? h('ol', { class: 'trace-children' }, kids.get(c.id).map(renderNode)) : null);
    replace(this.listEl, h('ol', { class: 'trace-list' }, roots.map(renderNode)));
    if (this.focusCall) {
      const el = document.getElementById(`call-${this.focusCall}`);
      this.focusCall = null;
      if (el) requestAnimationFrame(() => { el.scrollIntoView({ block: 'center' }); el.classList.add('flash'); });
    }
  }

  row(c) {
    const open = this.expanded.has(c.id);
    const body = h('div', { class: 'call-body' });
    const btn = h('button', { class: 'call-toggle', type: 'button', 'aria-expanded': String(open) },
      h('span', { class: `role-badge role-${c.agent}` }, (ROLES[c.agent] || {}).label || titleCase(c.agent)),
      h('span', { class: 'call-purpose' }, c.purpose || '—'),
      c.provider === 'mastra' ? h('span', { class: 'role-badge role-mastra', title: 'Ran in the Mastra companion' }, 'Mastra') : null,
      h('span', { class: 'call-meta small muted' }, fmtDuration(c.duration_ms)),
      c.input_tokens != null || c.output_tokens != null ? h('span', { class: 'call-meta small muted', title: 'Tokens reported by the model runtime' }, `${fmtNum(c.input_tokens ?? 0)}→${fmtNum(c.output_tokens ?? 0)} tok`) : null,
      c.ok ? pill('ok', 'OK') : pill('failed', 'Failed'));
    btn.addEventListener('click', () => {
      if (this.expanded.has(c.id)) this.expanded.delete(c.id); else this.expanded.add(c.id);
      const nowOpen = this.expanded.has(c.id);
      btn.setAttribute('aria-expanded', String(nowOpen));
      body.hidden = !nowOpen;
      if (nowOpen) this.fillBody(c, body);
    });
    body.hidden = !open;
    if (open) this.fillBody(c, body);
    return h('div', { class: ['call', !c.ok && 'call-failed'], id: `call-${c.id}` },
      btn,
      this.dev ? h('p', { class: 'call-dev small mono muted' },
        `${c.id} · ${c.provider}/${c.model} · ${fmtDate(c.started_at, true)} · prompt ${fmtNum(c.prompt_chars)} chars · response ${fmtNum(c.response_chars)} chars`) : null,
      (c.input_files || []).length ? h('p', { class: 'call-files small' }, h('span', { class: 'muted' }, 'Context given: '),
        c.input_files.map((f, i) => [i ? ', ' : '', h('button', { class: 'link mono', type: 'button', onclick: () => this.ctx.openFile(f) }, f)])) : null,
      c.error ? h('p', { class: 'small tone-text-bad' }, c.error) : null,
      body);
  }

  async fillBody(c, body) {
    if (this.cache.has(c.id)) { replace(body, this.bodyContent(this.cache.get(c.id))); return; }
    replace(body, spinner('Loading call'));
    try {
      const full = await api.call(c.id);
      this.cache.set(c.id, full);
      replace(body, this.bodyContent(full));
    } catch (e) {
      replace(body, errorBox(e.message));
    }
  }

  bodyContent(full) {
    const block = (title, text) => h('details', { class: 'call-block', open: title === 'Response' },
      h('summary', null, title, h('span', { class: 'muted small' }, ` · ${fmtNum((text || '').length)} chars`)),
      h('pre', { class: 'output' }, text || '(empty)'));
    return [
      this.dev ? block('System instructions', full.system) : null,
      this.dev ? block('Prompt (untrusted repository content is delimited inside)', full.prompt) : null,
      block('Response', full.response),
      this.dev ? null : h('p', { class: 'small muted' }, 'Turn on developer mode to see the full system instructions and prompt.'),
    ];
  }
}
