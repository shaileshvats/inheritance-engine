// Restoration: issues (essential vs improvement), repair controls, repair timeline.
import { h, replace, pill, emptyState, fmtDate, shortSha, sig, titleCase, diffView, details, markdown } from '../dom.js';
import { api } from '../api.js';
import { evidenceList, verificationSummary } from '../components.js';
import { Section } from '../section.js';

const EXPLAIN_PROMPT = 'Before changing anything, explain the issues you found: what is broken, the evidence for each, which are essential to restore the original behaviour, and what you would fix first.';

const REVIEW_LABEL = { approve: 'Approved', request_changes: 'Changes requested', reject: 'Rejected' };

export class RestorationTab {
  constructor(ctx) {
    this.ctx = ctx;
    this.selected = new Set();
  }

  mount() {
    this.controlsEl = h('div', { class: 'restore-controls card' });
    this.issuesEl = h('div', { class: 'issues' });
    this.repairsEl = h('div', { class: 'repairs' });
    this.controls = new Section(this.controlsEl, (d) => this.renderControls(d));
    this.issues = new Section(this.issuesEl, (d) => this.renderIssues(d));
    this.repairs = new Section(this.repairsEl, (d) => this.renderRepairs(d));
    return h('div', { class: 'restoration' },
      h('div', { class: 'arch-head' },
        h('h2', null, 'Restoration'),
        h('p', { class: 'small muted' }, 'Repairs happen only in the successor workspace, one commit per repair. Each is independently reviewed and verified; failed or rejected repairs are reverted and kept on record.')),
      this.controlsEl,
      h('h3', null, 'Issues'),
      this.issuesEl,
      h('h3', null, 'Repair history'),
      this.repairsEl);
  }

  update(d) {
    this.d = d;
    const busy = this.ctx.jobActive();
    const openIds = new Set((d.issues || []).filter((i) => i.status === 'open').map((i) => i.id));
    [...this.selected].forEach((id) => { if (!openIds.has(id)) this.selected.delete(id); });
    this.controls.update(d, sig([busy, d.execution_mode, d.active_job && d.active_job.kind, [...this.selected], (d.issues || []).map((i) => [i.id, i.status, i.category])]));
    this.issues.update(d, sig([d.issues, busy]));
    this.repairs.update(d, sig([d.repairs, d.issues && d.issues.map((i) => [i.id, i.title]), (d.verifications || []).map((v) => [v.id, v.status, v.summary])]));
    if (this.pendingIssue) this.focus({ issue: this.pendingIssue });
  }

  focus(f) {
    if (!f || !f.issue) return;
    const el = document.getElementById(`issue-${f.issue}`) || document.getElementById(`repair-${f.issue}`);
    if (!el) { this.pendingIssue = f.issue; return; }
    this.pendingIssue = null;
    requestAnimationFrame(() => { el.scrollIntoView({ block: 'center' }); el.classList.add('flash'); });
  }

  renderControls(d) {
    const busy = this.ctx.jobActive();
    const open = (d.issues || []).filter((i) => i.status === 'open');
    const essential = open.filter((i) => i.category === 'essential');
    const start = (mode, ids) => this.ctx.startJob(api.restore(this.ctx.id, mode, ids));
    const btn = (label, cls, onclick, disabled, title) => h('button', { class: cls, type: 'button', onclick, disabled, title }, label);
    const noExec = d.execution_mode === 'none';
    return [
      h('div', { class: 'restore-buttons' },
        btn('Explain first', 'btn', () => this.ctx.goTab('overview', { prompt: EXPLAIN_PROMPT }), busy || !open.length, 'Ask the Lead Engineer to explain the issues before anything changes'),
        btn(`Fix essential (${essential.length})`, 'btn btn-primary', () => start('essential'), busy || !essential.length),
        btn(`Fix everything (${open.length})`, 'btn', () => start('fix_all'), busy || !open.length, 'Routine, reversible repairs proceed in one batch without further prompts'),
        btn(`Fix selected (${this.selected.size})`, 'btn', () => start('selected', [...this.selected]), busy || !this.selected.size)),
      h('p', { class: 'small muted' },
        busy ? `A ${titleCase(d.active_job.kind).toLowerCase()} job is running. Repair controls are available when it finishes.`
          : noExec ? 'Execution mode is "No execution": repairs can be proposed and reviewed, but not verified by running tests. Change the execution mode in the header to verify.'
            : 'Nothing is pushed or published. The original repository is never modified.'),
    ];
  }

  renderIssues(d) {
    const issues = d.issues || [];
    if (!issues.length) {
      return emptyState(d.report ? 'No issues recorded.' : 'No diagnosis yet.',
        d.report ? 'The Restorer found nothing to repair, or the investigation has not reached diagnosis.' : 'Issues appear after the investigation diagnoses the project.');
    }
    const busy = this.ctx.jobActive();
    const group = (cat, title, blurb) => {
      const list = issues.filter((i) => i.category === cat);
      if (!list.length) return null;
      return h('section', { class: `issue-group issue-group-${cat}` },
        h('div', { class: 'issue-group-head' }, h('h4', null, `${title} (${list.length})`), h('p', { class: 'small muted' }, blurb)),
        list.map((i) => this.issueCard(i, busy)));
    };
    return [
      group('essential', 'Essential repairs', 'Needed to restore the original behaviour.'),
      group('improvement', 'Optional improvements', 'Modernisation and quality; not required for restoration.'),
    ];
  }

  issueCard(i, busy) {
    const checkbox = i.status === 'open' ? h('input', { type: 'checkbox', checked: this.selected.has(i.id), disabled: busy, 'aria-label': `Select ${i.title}` }) : null;
    if (checkbox) checkbox.addEventListener('change', () => {
      if (checkbox.checked) this.selected.add(i.id); else this.selected.delete(i.id);
      this.controls.invalidate(); this.controls.update(this.d);
    });
    const repairs = (this.d.repairs || []).filter((r) => r.issue_id === i.id);
    return h('article', { class: ['issue', `issue-${i.status}`], id: `issue-${i.id}` },
      h('div', { class: 'issue-head' },
        checkbox ? h('label', { class: 'issue-check' }, checkbox) : null,
        h('h5', null, i.title),
        h('div', { class: 'issue-pills' }, pill(i.severity, `${titleCase(i.severity)} severity`), pill(i.status), h('span', { class: 'small muted' }, `confidence ${i.confidence || '—'} · via ${titleCase(i.source || 'restorer')}`))),
      h('p', null, i.diagnosis),
      i.proposed_fix ? h('p', { class: 'small' }, h('strong', null, 'Proposed fix: '), i.proposed_fix) : null,
      evidenceList(i.evidence, this.ctx, `iss-ev-${i.id}`),
      repairs.length ? h('p', { class: 'small' }, 'Repair attempts: ', repairs.map((r, k) => [k ? ', ' : '',
        h('button', { class: 'link', type: 'button', onclick: () => this.focus({ issue: r.id }) }, `${titleCase(r.status)} ${fmtDate(r.created_at, true)}`)])) : null,
      i.status !== 'open' ? this.reopenForm(i, busy) : null);
  }

  /** The inheritor can disagree with the engine: reopen an issue with their own finding. */
  reopenForm(i, busy) {
    const note = h('textarea', { rows: 3, class: 'input', 'aria-label': `What did you find about ${i.title}?`, placeholder: 'What did you observe? e.g. "curried() behaves differently for functions with *args than the original did."' });
    const status = h('p', { class: 'small muted', role: 'status', 'aria-live': 'polite' });
    const submit = h('button', { class: 'btn btn-secondary', type: 'button', disabled: busy }, 'Reopen issue');
    submit.addEventListener('click', async () => {
      if (!note.value.trim()) { status.textContent = 'Describe what you found first.'; return; }
      submit.disabled = true;
      try {
        await api.reopenIssue(this.d.id, i.id, note.value.trim());
        status.textContent = 'Reopened. Use "Fix selected" to restore it again with your note.';
        this.ctx.refresh && this.ctx.refresh();
      } catch (e) { status.textContent = `Could not reopen: ${e.message}`; submit.disabled = false; }
    });
    return h('details', { class: 'reopen small' }, h('summary', null, 'Disagree? Reopen with your finding'), note, h('div', { class: 'row' }, submit), status);
  }

  renderRepairs(d) {
    const repairs = (d.repairs || []).slice().sort((a, b) => String(b.created_at).localeCompare(String(a.created_at)));
    if (!repairs.length) return emptyState('No repairs attempted yet.', 'Choose "Fix essential" or "Fix everything" above to let the Restorer work through the issues in batches.');
    const issues = new Map((d.issues || []).map((i) => [i.id, i]));
    const vers = new Map((d.verifications || []).map((v) => [v.id, v]));
    return h('ol', { class: 'timeline' }, repairs.map((r, idx) => this.repairItem(r, issues.get(r.issue_id), vers, idx === 0)));
  }

  probeBlock(r) {
    const p = r.probe;
    const label = { preserved: 'Behaviour preserved', changed: 'Behaviour changed', inconclusive: 'Inconclusive' }[p.verdict] || titleCase(p.verdict);
    const pillState = { preserved: 'passed', changed: 'failed', inconclusive: 'not_run' }[p.verdict] || p.verdict;
    return h('div', { class: ['review', `probe-${p.verdict}`] },
      h('div', { class: 'review-head' },
        h('h6', null, 'Behaviour probe'),
        pill(pillState, label),
        p.legacy_image ? h('span', { class: 'small' }, `original on ${p.legacy_image} vs successor on ${p.modern_image}`) : null,
        p.tester_call_id ? h('button', { class: 'link small', type: 'button', onclick: () => this.ctx.goTab('agents', { call: p.tester_call_id }) }, 'trace') : null),
      p.note ? h('p', { class: 'small muted' }, p.note) : null,
      (p.probes || []).length ? h('table', { class: 'table small' },
        h('thead', null, h('tr', null, h('th', { scope: 'col' }, 'Probe'), h('th', { scope: 'col' }, 'Original'), h('th', { scope: 'col' }, 'Successor'), h('th', { scope: 'col' }, 'Result'))),
        h('tbody', null, p.probes.map((x) => h('tr', null,
          h('td', null, h('details', null, h('summary', null, x.name), h('pre', { class: 'mono' }, x.code))),
          h('td', { class: 'mono' }, x.original_output || '—'),
          h('td', { class: 'mono' }, x.successor_output || '—'),
          h('td', null, pill({ preserved: 'passed', changed: 'failed' }[x.verdict] || 'not_run', titleCase(x.verdict))))))) : null);
  }

  repairItem(r, issue, vers, latest) {
    const rev = r.review;
    const verLink = (id, label) => {
      if (!id) return h('span', { class: 'muted' }, `${label}: not run`);
      const v = vers.get(id);
      return h('button', { class: 'link', type: 'button', onclick: () => this.ctx.goTab('verification', { verification: id }) },
        `${label}: `, v ? `${titleCase(v.status)} (${verificationSummary(v)})` : id);
    };
    return h('li', { class: ['timeline-item', `repair-${r.status}`], id: `repair-${r.id}` },
      h('span', { class: 'timeline-dot', 'aria-hidden': 'true' }),
      h('article', { class: 'repair card' },
        h('div', { class: 'repair-head' },
          pill(r.status),
          h('h5', null, issue ? issue.title : r.issue_id),
          h('span', { class: 'small muted' }, fmtDate(r.created_at, true)),
          r.commit ? h('span', { class: 'mono small' }, `commit ${shortSha(r.commit)}`) : null),
        r.note ? h('p', { class: `callout ${r.status === 'applied' ? 'callout-info' : 'callout-warn'} small` }, r.note) : null,
        r.rationale ? h('div', null, h('h6', null, 'Restorer rationale'), markdown(r.rationale)) : null,
        rev ? h('div', { class: ['review', `review-${rev.verdict}`] },
          h('div', { class: 'review-head' },
            h('h6', null, 'Independent review'),
            pill(rev.verdict, REVIEW_LABEL[rev.verdict] || titleCase(rev.verdict)),
            h('span', { class: 'small' }, `root cause addressed: ${rev.root_cause_addressed ? 'yes' : 'no'} · regression risk: ${rev.regression_risk || '—'}`),
            rev.call_id ? h('button', { class: 'link small', type: 'button', onclick: () => this.ctx.goTab('agents', { call: rev.call_id }) }, 'trace') : null),
          (rev.concerns || []).length ? h('ul', { class: 'small' }, rev.concerns.map((c) => h('li', null, c))) : h('p', { class: 'small muted' }, 'No concerns raised.'))
          : h('p', { class: 'small muted' }, 'Not reviewed.'),
        h('div', { class: 'repair-verify small' }, verLink(r.verification_before, 'Before'), h('span', { 'aria-hidden': 'true' }, ' → '), verLink(r.verification_after, 'After')),
        r.probe ? this.probeBlock(r) : null,
        (r.files || []).length ? h('p', { class: 'small' }, 'Files: ', r.files.map((f, k) => [k ? ', ' : '',
          h('button', { class: 'link mono', type: 'button', onclick: () => this.ctx.openFile(f) }, f)])) : null,
        details(`diff-${r.id}`, this.ctx.openSet, 'Diff', diffView(r.diff), { openByDefault: latest })));
  }
}
