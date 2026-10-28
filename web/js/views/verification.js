// Verification: recorded command runs and a form to run a user-authored command.
import { h, pill, emptyState, fmtDate, fmtDuration, fmtNum, shortSha, sig, details, errorBox, titleCase } from '../dom.js';
import { api } from '../api.js';
import { verificationSummary } from '../components.js';
import { Section } from '../section.js';

export class VerificationTab {
  constructor(ctx) { this.ctx = ctx; }

  mount() {
    this.formEl = h('div', { class: 'verify-form card' });
    this.listEl = h('div', { class: 'ver-list' });
    this.form = new Section(this.formEl, (d) => this.renderForm(d));
    this.list = new Section(this.listEl, (d) => this.renderList(d));
    return h('div', { class: 'verification' },
      h('div', { class: 'arch-head' },
        h('h2', null, 'Verification'),
        h('p', { class: 'small muted' }, 'Every result below is a real command execution with its exit code and output. Compiling is not the same as working; "not run" is never shown as success.')),
      this.formEl,
      this.listEl);
  }

  update(d) {
    this.d = d;
    this.form.update(d, sig([d.execution_mode, this.ctx.jobActive(), (d.verifications || [])[0] && d.verifications[0].command]));
    this.list.update(d, sig(d.verifications));
    if (this.pending) this.focus(this.pending);
  }

  focus(f) {
    if (!f || !f.verification) return;
    const el = document.getElementById(`ver-${f.verification}`);
    if (!el) { this.pending = f; return; }
    this.pending = null;
    this.ctx.openSet.add(`out-${f.verification}`);
    const det = el.querySelector('details');
    if (det) det.open = true;
    requestAnimationFrame(() => { el.scrollIntoView({ block: 'center' }); el.classList.add('flash'); });
  }

  latest(d) {
    const v = (d.verifications || []).slice().sort((a, b) => String(b.started_at).localeCompare(String(a.started_at)));
    return v.find((x) => x.command && x.status !== 'not_run') || v[0];
  }

  renderForm(d) {
    const busy = this.ctx.jobActive();
    const last = this.latest(d);
    const input = h('input', { class: 'input mono', id: 'verify-cmd', value: (last && last.command) || '', placeholder: 'Leave empty to use the detected test command', autocomplete: 'off', spellcheck: 'false' });
    const run = h('button', { class: 'btn btn-primary', type: 'submit', disabled: busy || d.execution_mode === 'none' }, 'Run verification');
    const msg = h('div');
    const form = h('form', { class: 'verify-row' },
      h('label', { for: 'verify-cmd', class: 'meta-label' }, 'Command'),
      input, run);
    form.addEventListener('submit', async (e) => {
      e.preventDefault();
      run.disabled = true;
      const job = await this.ctx.startJob(api.verify(this.ctx.id, input.value.trim() || undefined));
      if (!job) { run.disabled = false; msg.replaceChildren(errorBox('Could not start verification.')); }
    });
    const env = d.execution_mode === 'docker' ? 'Runs in the Docker sandbox: network disabled, resource limits, no host credentials.'
      : d.execution_mode === 'native' ? 'Runs natively on this machine inside the successor workspace with a cleared environment.'
        : 'Execution mode is "No execution", so nothing can be run. Change it in the header to verify.';
    return [form,
      h('p', { class: ['small', d.execution_mode === 'native' ? 'tone-text-warn' : 'muted'] }, env, busy ? ' A job is running; verification is available when it finishes.' : ''),
      msg];
  }

  renderList(d) {
    const runs = (d.verifications || []).slice().sort((a, b) => String(b.started_at).localeCompare(String(a.started_at)));
    if (!runs.length) return emptyState('No verification runs yet.', 'A baseline run happens during investigation when an execution mode is chosen. You can also run a command above.');
    return h('ol', { class: 'ver-runs' }, runs.map((v, i) => this.run(v, i === 0)));
  }

  run(v, first) {
    const s = v.summary;
    return h('li', { class: ['ver-run card', `ver-${v.status}`], id: `ver-${v.id}` },
      h('div', { class: 'ver-head' },
        pill(v.status, v.status === 'not_run' ? 'Not run' : titleCase(v.status)),
        h('h4', null, v.label || 'Verification'),
        h('span', { class: 'small muted' }, fmtDate(v.started_at, true))),
      v.command ? h('pre', { class: 'cmd' }, h('span', { class: 'cmd-prompt', 'aria-hidden': 'true' }, '$ '), v.command) : null,
      v.reason ? h('p', { class: 'small' }, h('strong', null, 'Reason: '), v.reason) : null,
      h('dl', { class: 'ver-facts' },
        fact('Mode', v.mode || '—'),
        fact('Environment', v.environment || '—'),
        fact('Network', v.network || '—'),
        fact('Exit code', v.exit_code == null ? '—' : String(v.exit_code)),
        fact('Duration', fmtDuration(v.duration_ms)),
        fact('Workspace', shortSha(v.workspace_rev))),
      s ? h('div', { class: 'ver-counts', 'aria-label': verificationSummary(v) },
        count('passed', s.passed, 'good'), count('failed', s.failed, 'bad'), count('errors', s.errors, 'bad'), count('skipped', s.skipped, 'muted'))
        : (v.status !== 'not_run' ? h('p', { class: 'small muted' }, 'Test counts could not be parsed from the output; the exit code is the result.') : null),
      v.output_tail ? details(`out-${v.id}`, this.ctx.openSet, 'Output (last lines)', h('pre', { class: 'output' }, v.output_tail), { openByDefault: first && v.status !== 'passed' }) : null);
  }
}

function fact(label, value) { return h('div', null, h('dt', null, label), h('dd', { class: 'mono' }, value)); }
function count(label, n, t) { return h('span', { class: `count tone-${n ? t : 'muted'}` }, h('strong', null, fmtNum(n || 0)), ` ${label}`); }
