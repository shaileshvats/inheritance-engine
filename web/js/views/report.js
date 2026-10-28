// Archaeology report: purpose, capabilities, components, reviewer challenges.
import { h, pill, markdown, emptyState, fmtDate, titleCase, sig } from '../dom.js';
import { claimCard, evidenceList } from '../components.js';
import { Section } from '../section.js';
import { api } from '../api.js';

const VERDICT_LABEL = { agree: 'Agrees', disagree: 'Disagrees', insufficient_evidence: 'Insufficient evidence' };

export class ReportTab {
  constructor(ctx) { this.ctx = ctx; }

  mount() {
    this.el = h('div', { class: 'report' });
    this.section = new Section(this.el, (d) => this.render(d));
    return this.el;
  }

  update(d) { this.section.update(d, sig([d.report, !!d.active_job, (d.issues || []).map((i) => [i.id, i.title])])); }

  focus(f) {
    if (f && f.claim) requestAnimationFrame(() => {
      const el = document.getElementById(`claim-${f.claim}`);
      if (el) { el.scrollIntoView({ block: 'center' }); el.classList.add('flash'); }
    });
  }

  render(d) {
    const r = d.report;
    if (!r) {
      const running = d.active_job && d.active_job.kind === 'investigate';
      return emptyState(running ? 'The agent team is investigating.' : 'No archaeology report yet.',
        running
          ? 'The Lead Engineer delegates scoped tasks to the Historian, Restorer and Reviewer. Their findings appear here with evidence as soon as the report is synthesised.'
          : 'Run an investigation to have the agent team reconstruct the project\'s purpose and architecture.',
        running ? null : h('button', { class: 'btn btn-primary', type: 'button', onclick: () => this.ctx.startJob(api.investigate(this.ctx.id)) }, 'Run investigation'));
    }

    const challenges = r.reviewer_challenges || [];
    const objections = challenges.filter((c) => c.verdict !== 'agree');
    const lookup = this.targetLookup(d);

    return [
      h('div', { class: 'report-head' },
        h('h2', null, 'Archaeology report'),
        h('p', { class: 'small muted' }, `Synthesised by the Lead Engineer ${r.generated_at ? fmtDate(r.generated_at, true) : ''}. `,
          'Every claim is marked verified, inferred or unknown, and every quote is checked against the source by the engine, not the model.')),

      objections.length ? h('section', { class: 'callout callout-warn challenges-top', 'aria-labelledby': 'obj-title' },
        h('h3', { id: 'obj-title' }, `The Reviewer challenged ${objections.length} finding${objections.length === 1 ? '' : 's'}`),
        h('p', { class: 'small' }, 'Disagreements are shown, not hidden. Weigh the evidence before acting on these findings.'),
        h('ul', { class: 'challenge-list' }, objections.map((c) => this.challenge(c, lookup)))) : null,

      r.lead_summary ? h('section', { class: 'card' }, h('h3', null, 'Lead Engineer summary'), markdown(r.lead_summary)) : null,

      h('section', { class: 'report-section' }, h('h3', null, 'Original purpose'),
        r.purpose ? claimCard(r.purpose, this.ctx) : h('p', { class: 'muted' }, 'Unknown.')),

      claimsSection('Capabilities', r.capabilities, this.ctx),

      h('section', { class: 'report-section' }, h('h3', null, 'Essential components'),
        (r.components || []).length
          ? h('div', { class: 'component-list' }, r.components.map((c, i) => h('article', { class: 'claim' },
            h('div', { class: 'claim-head' }, pill(c.status || 'unknown'),
              h('button', { class: 'link mono', type: 'button', onclick: () => this.ctx.openFile(c.path) }, c.path),
              h('button', { class: 'link small', type: 'button', onclick: () => this.ctx.goTab('architecture', { path: c.path }) }, 'show in graph')),
            h('p', { class: 'claim-text' }, c.role || '—'),
            evidenceList(c.evidence, this.ctx, `comp-${c.path}-${i}`))))
          : h('p', { class: 'muted' }, 'None identified.')),

      claimsSection('Data flows', r.data_flows, this.ctx),
      claimsSection('External services', r.external_services, this.ctx),

      r.build_instructions ? h('section', { class: 'report-section' }, h('h3', null, 'How to build and run'), markdown(r.build_instructions)) : null,

      h('section', { class: 'report-section' }, h('h3', null, 'Unknowns'),
        (r.unknowns || []).length
          ? h('ul', { class: 'unknowns' }, r.unknowns.map((u) => h('li', null, u)))
          : h('p', { class: 'muted' }, 'None recorded.')),

      challenges.length ? h('section', { class: 'report-section' }, h('h3', null, 'All reviewer verdicts'),
        h('ul', { class: 'challenge-list' }, challenges.map((c) => this.challenge(c, lookup)))) : null,
    ];
  }

  targetLookup(d) {
    const map = new Map();
    const r = d.report || {};
    const add = (c) => { if (c && c.id) map.set(c.id, { text: c.text, kind: 'claim' }); };
    add(r.purpose);
    [r.capabilities, r.data_flows, r.external_services].forEach((list) => (list || []).forEach(add));
    (d.issues || []).forEach((i) => map.set(i.id, { text: i.title, kind: 'issue' }));
    return map;
  }

  challenge(c, lookup) {
    const target = lookup.get(c.target);
    return h('li', { class: ['challenge', `verdict-${c.verdict}`] },
      pill(c.verdict, VERDICT_LABEL[c.verdict] || titleCase(c.verdict)),
      h('div', null,
        h('p', { class: 'challenge-target' },
          target ? h('button', {
            class: 'link', type: 'button',
            onclick: () => target.kind === 'issue' ? this.ctx.goTab('restoration', { issue: c.target }) : this.focus({ claim: c.target }),
          }, target.text) : h('span', { class: 'mono' }, c.target)),
        h('p', { class: 'small' }, c.reason || '')));
  }
}

function claimsSection(title, claims, ctx) {
  return h('section', { class: 'report-section' }, h('h3', null, title),
    (claims || []).length
      ? h('div', { class: 'claim-list' }, claims.map((c) => claimCard(c, ctx)))
      : h('p', { class: 'muted' }, 'None identified.'));
}
