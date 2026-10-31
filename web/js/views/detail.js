// Inheritance detail: header, job panel, tabs.
import { h, replace, pill, shortSha, copyButton, errorBox, spinner, titleCase, fmtDate, relTime, announce, sig, details } from '../dom.js';
import { api, Poller, ACTIVE_JOB } from '../api.js';
import { lifecycle, chooseExecutionMode, verificationBadge, openFileViewer } from '../components.js';
import { Section } from '../section.js';
import { OverviewTab } from './overview.js';
import { ReportTab } from './report.js';
import { ArchitectureTab } from './architecture.js';
import { RestorationTab } from './restoration.js';
import { VerificationTab } from './verification.js';
import { PassportTab } from './passport.js';
import { AgentsTab } from './agents.js';
import { RecordTab } from './record.js';

const TABS = [
  ['overview', 'Overview', OverviewTab],
  ['report', 'Archaeology', ReportTab],
  ['architecture', 'Architecture', ArchitectureTab],
  ['restoration', 'Restoration', RestorationTab],
  ['verification', 'Verification', VerificationTab],
  ['passport', 'Trust passport', PassportTab],
  ['agents', 'Agent trace', AgentsTab],
  ['record', 'Record & lineage', RecordTab],
];

const STEP_ICON = { pending: '○', running: '◐', done: '●', skipped: '–', failed: '✕' };

export class DetailView {
  constructor(id, tab) {
    this.id = id;
    this.tabName = TABS.some(([t]) => t === tab) ? tab : 'overview';
    this.detail = null;
    this.ctx = {
      id,
      detail: null,
      openSet: new Set(),
      focus: null,
      refresh: () => this.poller && this.poller.now(),
      openFile: (path, start, end, side) => openFileViewer(id, path, start, end, side),
      goTab: (tab, focus) => { this.ctx.focus = focus || null; location.hash = `#/i/${encodeURIComponent(id)}/${tab}`; },
      jobActive: () => !!(this.detail && this.detail.active_job && ACTIVE_JOB.has(this.detail.active_job.status)),
      startJob: (promise) => this.startJob(promise),
      status: null,
    };
    api.status().then((s) => { this.ctx.status = s; }).catch(() => {});
    this.lastStepKey = null;
  }

  mount() {
    this.root = h('div', { class: 'page detail' });
    this.headerEl = h('header', { class: 'detail-header' }, h('h1', null, 'Loading…'), spinner());
    this.jobEl = h('section', { class: 'job-slot', 'aria-label': 'Background job' });
    this.navEl = h('nav', { class: 'tabs', 'aria-label': 'Inheritance sections' });
    this.panelEl = h('div', { class: 'tab-panel' });
    this.errorEl = h('div');
    this.root.append(this.headerEl, this.errorEl, this.jobEl, this.navEl, this.panelEl);

    this.header = new Section(this.headerEl, (d) => this.renderHeader(d));
    this.jobSection = new Section(this.jobEl, (d) => this.renderJob(d));
    this.renderNav();
    this.mountTab();

    this.poller = new Poller(() => this.load(), () => (this.ctx.jobActive() ? 2000 : 8000)).start();
    return this.root;
  }

  destroy() {
    if (this.poller) this.poller.stop();
    if (this.tab && this.tab.destroy) this.tab.destroy();
  }

  setTab(tab) {
    const next = TABS.some(([t]) => t === tab) ? tab : 'overview';
    if (next === this.tabName) { this.applyFocus(); return; }
    this.tabName = next;
    this.renderNav();
    this.mountTab();
  }

  renderNav() {
    replace(this.navEl, TABS.map(([t, label]) => h('a', {
      class: ['tab', t === this.tabName && 'active'],
      href: `#/i/${encodeURIComponent(this.id)}/${t}`,
      'aria-current': t === this.tabName ? 'page' : undefined,
    }, label)));
  }

  mountTab() {
    if (this.tab && this.tab.destroy) this.tab.destroy();
    const Cls = TABS.find(([t]) => t === this.tabName)[2];
    this.tab = new Cls(this.ctx);
    replace(this.panelEl, this.tab.mount());
    if (this.detail) this.tab.update(this.detail);
    this.applyFocus();
  }

  /** Hand a one-shot focus request (e.g. "open verification run X") to the tab. */
  applyFocus() {
    const f = this.ctx.focus;
    this.ctx.focus = null;
    if (f && this.tab && this.tab.focus) this.tab.focus(f);
  }

  async load() {
    let d;
    try { d = await api.get(this.id); }
    catch (e) {
      replace(this.errorEl, errorBox(e.status === 404 ? 'This inheritance does not exist (it may have been removed).' : e.message));
      return;
    }
    replace(this.errorEl);
    this.detail = d;
    this.ctx.detail = d;
    this.header.update(d, sig([d.name, d.full_name, d.source_url, d.license, d.stage, d.execution_mode, d.original, d.successor, d.last_verification]));
    this.jobSection.update(d, sig([d.active_job, (d.jobs || []).slice(0, 5)]));
    this.announceJob(d.active_job);
    if (this.tab) this.tab.update(d);
  }

  announceJob(job) {
    if (!job) return;
    const key = `${job.id}:${job.step}:${job.status}`;
    if (key === this.lastStepKey) return;
    if (this.lastStepKey) announce(`${titleCase(job.kind)} job ${job.status}${job.step ? `: ${titleCase(job.step)}` : ''}`);
    this.lastStepKey = key;
  }

  async startJob(promise) {
    try {
      const job = await promise;
      if (job && job.kind) announce(`${titleCase(job.kind)} job started`);
      this.ctx.refresh();
      return job;
    } catch (e) {
      replace(this.errorEl, errorBox(e.message));
      return null;
    }
  }

  // ---------- Header ----------

  renderHeader(d) {
    const orig = d.original || {};
    const integ = orig.integrity;
    const succ = d.successor || {};
    const integrity = integ
      ? (integ.ok
        ? h('span', { class: 'badge tone-good', title: `Checked ${fmtDate(integ.checked_at, true)}` }, '✓ Original untouched ', h('span', { class: 'mono' }, shortSha(integ.head)))
        : h('span', { class: 'badge tone-bad', role: 'alert' }, '⚠ Original changed! HEAD is ', h('span', { class: 'mono' }, shortSha(integ.head))))
      : h('span', { class: 'badge tone-muted' }, 'Integrity not checked yet');

    return [
      h('div', { class: 'detail-title-row' },
        h('div', null,
          h('p', { class: 'eyebrow' }, h('a', { href: '#/' }, '← Dashboard')),
          h('h1', null, d.name || d.full_name),
          h('p', { class: 'detail-sub' },
            h('a', { href: d.source_url, target: '_blank', rel: 'noopener noreferrer', class: 'mono' }, d.full_name, ' ↗'),
            h('span', { class: 'sep', 'aria-hidden': 'true' }, '·'),
            `Licence: ${d.license || 'none declared'}`,
            h('span', { class: 'sep', 'aria-hidden': 'true' }, '·'),
            'Original ', h('span', { class: 'mono' }, shortSha(d.original_commit || orig.commit)),
            (d.original_commit || orig.commit) ? copyButton(d.original_commit || orig.commit, 'Copy SHA') : null)),
        h('div', { class: 'detail-health' }, verificationBadge(d.last_verification), integrity)),
      h('div', { class: 'detail-meta-row' },
        this.successorBlock(d, succ),
        this.modeBlock(d)),
      lifecycle(d.stage),
    ];
  }

  successorBlock(d, succ) {
    const wrap = h('div', { class: 'meta-block' });
    const show = () => replace(wrap,
      h('span', { class: 'meta-label' }, 'Successor'),
      h('strong', null, succ.name || '—'),
      h('button', { class: 'btn btn-ghost btn-xs', type: 'button', onclick: edit, 'aria-label': 'Rename successor' }, 'Rename'),
      h('span', { class: 'muted small' }, ' branch ', h('span', { class: 'mono' }, succ.branch || '—'),
        succ.head ? [' @ ', h('span', { class: 'mono' }, shortSha(succ.head))] : null,
        ` · ${succ.commits_ahead ?? 0} commit${succ.commits_ahead === 1 ? '' : 's'} ahead of original`));
    const edit = () => {
      const input = h('input', { class: 'input input-sm', value: succ.name || '', 'aria-label': 'Successor name', maxlength: 100 });
      const save = h('button', { class: 'btn btn-primary btn-xs', type: 'submit' }, 'Save');
      const cancel = h('button', { class: 'btn btn-xs', type: 'button', onclick: () => { this.header.unlock(); show(); } }, 'Cancel');
      const form = h('form', { class: 'inline-form' }, h('span', { class: 'meta-label' }, 'Successor'), input, save, cancel);
      form.addEventListener('submit', async (e) => {
        e.preventDefault();
        const name = input.value.trim();
        if (!name) return;
        save.disabled = true;
        try { await api.renameSuccessor(this.id, name); succ.name = name; this.header.unlock(); show(); this.ctx.refresh(); }
        catch (err) { save.disabled = false; form.append(errorBox(err.message)); }
      });
      this.header.lock();
      replace(wrap, form);
      input.focus();
      input.select();
    };
    show();
    return wrap;
  }

  modeBlock(d) {
    const labels = { docker: 'Docker sandbox', native: 'Native (this machine)', none: 'No execution' };
    const btn = h('button', { class: 'btn btn-ghost btn-xs', type: 'button' }, 'Change');
    btn.addEventListener('click', async () => {
      const mode = await chooseExecutionMode(d.full_name, d.execution_mode, {
        confirmLabel: 'Use this mode',
        intro: 'Changing where inherited code runs takes effect for the next verification or repair. The engine never switches this on its own.',
      });
      if (!mode || mode === d.execution_mode) return;
      try { await api.setExecutionMode(this.id, mode); this.ctx.refresh(); }
      catch (e) { replace(this.errorEl, errorBox(e.message)); }
    });
    return h('div', { class: 'meta-block' },
      h('span', { class: 'meta-label' }, 'Execution'),
      h('strong', { class: d.execution_mode === 'native' ? 'tone-text-warn' : '' }, labels[d.execution_mode] || d.execution_mode || '—'),
      btn);
  }

  // ---------- Job panel ----------

  renderJob(d) {
    const job = d.active_job || (d.jobs || []).find((j) => ['failed', 'interrupted', 'paused'].includes(j.status));
    const recent = (d.jobs || []).filter((j) => !job || j.id !== job.id).slice(0, 5);
    const parts = [];
    if (job) parts.push(this.jobCard(job));
    if (recent.length) {
      parts.push(details('recent-jobs', this.ctx.openSet, `Recent jobs (${recent.length})`,
        h('ul', { class: 'job-history' }, recent.map((j) => h('li', null,
          pill(j.status), ' ', h('strong', null, titleCase(j.kind)),
          h('span', { class: 'muted small' }, ` · ${relTime(j.updated_at)}`),
          j.error ? h('span', { class: 'small tone-text-bad' }, ` — ${j.error}`) : null))),
        { className: 'recent-jobs' }));
    }
    return parts;
  }

  jobCard(job) {
    const action = (name, label, cls = 'btn btn-xs') => {
      const b = h('button', { class: cls, type: 'button' }, label);
      b.addEventListener('click', async () => {
        b.disabled = true;
        try { await api.jobAction(job.id, name); this.ctx.refresh(); }
        catch (e) { b.disabled = false; replace(this.errorEl, errorBox(e.message)); }
      });
      return b;
    };
    const controls = [];
    if (job.status === 'running') controls.push(action('pause', 'Pause'), action('cancel', 'Cancel'));
    else if (job.status === 'queued') controls.push(action('cancel', 'Cancel'));
    else if (job.status === 'paused') controls.push(action('resume', 'Resume', 'btn btn-primary btn-xs'), action('cancel', 'Cancel'));
    else if (job.status === 'interrupted' || job.status === 'failed') controls.push(action('resume', 'Resume from checkpoint', 'btn btn-primary btn-xs'));

    const steps = job.steps || [];
    const done = steps.filter((s) => s.status === 'done' || s.status === 'skipped').length;
    return h('div', { class: ['job-card', `job-${job.status}`] },
      h('div', { class: 'job-head' },
        h('div', null,
          h('span', { class: 'meta-label' }, 'Job'),
          h('strong', null, titleCase(job.kind)), ' ', pill(job.status),
          h('span', { class: 'muted small' }, ` · step ${Math.min(done + 1, steps.length || 1)} of ${steps.length || '?'}`,
            job.step ? ` · ${titleCase(job.step)}` : '')),
        h('div', { class: 'job-controls' }, controls)),
      steps.length ? h('div', { class: 'job-progress', role: 'progressbar', 'aria-valuemin': 0, 'aria-valuemax': steps.length, 'aria-valuenow': done, 'aria-label': 'Job progress' },
        h('span', { style: { width: `${(done / steps.length) * 100}%` } })) : null,
      h('ol', { class: 'job-steps' }, steps.map((s) => h('li', { class: ['job-step', `step-${s.status}`] },
        h('span', { class: 'step-icon', 'aria-hidden': 'true' }, STEP_ICON[s.status] || '○'),
        h('span', { class: 'step-name' }, titleCase(s.name)),
        h('span', { class: 'sr-only' }, ` (${s.status})`),
        s.detail ? h('span', { class: 'step-detail muted small' }, s.detail) : null))),
      job.error ? h('p', { class: 'error-box', role: 'alert' }, job.error) : null);
  }
}
