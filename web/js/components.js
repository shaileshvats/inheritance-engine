// Shared UI pieces: dialogs, file viewer, evidence and claims, lifecycle.
import { h, replace, pill, tone, titleCase, details, spinner, errorBox, fmtNum, fmtDate, shortSha } from './dom.js';
import { api } from './api.js';

export const STAGES = ['discovered', 'inherited', 'investigating', 'understanding', 'restoring', 'verified', 'evolving'];

const CHECK_LABEL = {
  verified: 'Quote matches source',
  mismatch: 'Quote does not match source',
  missing_file: 'Cited file not found',
  out_of_range: 'Cited lines out of range',
  unchecked: 'Not checked',
};

const CHECK_ICON = { verified: '✓', mismatch: '⚠', missing_file: '✗', out_of_range: '✗', unchecked: '·' };

// ---------- Dialog helper ----------

function modal(titleText, body, footer, { wide = false, onClose } = {}) {
  const titleId = `dlg-${Math.random().toString(36).slice(2)}`;
  const closeBtn = h('button', { class: 'icon-btn', type: 'button', 'aria-label': 'Close dialog' }, '✕');
  const dlg = h('dialog', { class: ['dialog', wide && 'dialog-wide'], 'aria-labelledby': titleId },
    h('div', { class: 'dialog-head' }, h('h2', { id: titleId }, titleText), closeBtn),
    h('div', { class: 'dialog-body' }, body),
    footer ? h('div', { class: 'dialog-foot' }, footer) : null);
  closeBtn.addEventListener('click', () => dlg.close());
  dlg.addEventListener('close', () => { dlg.remove(); if (onClose) onClose(dlg.returnValue); });
  dlg.addEventListener('click', (e) => { if (e.target === dlg) dlg.close(); });
  document.body.append(dlg);
  dlg.showModal();
  return dlg;
}

// ---------- Inherit dialog ----------

const MODES = [
  { value: 'docker', title: 'Docker sandbox', tag: 'Recommended for unfamiliar code',
    body: 'Tests and builds run in a throwaway container with resource limits, no host credentials and networking disabled except for an explicit dependency-install step. Docker reduces risk; it is not an absolute guarantee.' },
  { value: 'native', title: 'Native', tag: 'Runs on this machine',
    body: 'Commands run directly on your computer inside the workspace folder with a cleared environment. Inherited code can still read and write anything your user account can. Only choose this for code you trust.', warn: true },
  { value: 'none', title: 'No execution', tag: 'Static analysis only',
    body: 'Nothing from the repository is executed. Archaeology and repair proposals still work, but nothing can be verified by running tests.' },
];

/** Ask the user to choose an execution mode explicitly. Resolves to the mode or null. */
export function chooseExecutionMode(fullName, current = 'docker', { confirmLabel = 'Inherit repository', intro } = {}) {
  return new Promise((resolve) => {
    let chosen = current;
    const name = `mode-${Math.random().toString(36).slice(2)}`;
    const warnBox = h('p', { class: 'callout callout-warn', hidden: chosen !== 'native' },
      'Native mode executes code written by someone else directly on your machine. Make sure you have read it first.');
    const options = MODES.map((m) => {
      const input = h('input', { type: 'radio', name, value: m.value, checked: m.value === chosen });
      input.addEventListener('change', () => { chosen = m.value; warnBox.hidden = chosen !== 'native'; });
      return h('label', { class: ['mode-option', m.warn && 'mode-option-warn'] },
        input,
        h('span', { class: 'mode-option-text' },
          h('span', { class: 'mode-option-title' }, m.title, h('span', { class: 'mode-option-tag' }, m.tag)),
          h('span', { class: 'mode-option-body' }, m.body)));
    });
    const confirm = h('button', { class: 'btn btn-primary', type: 'button' }, confirmLabel);
    const cancel = h('button', { class: 'btn', type: 'button' }, 'Cancel');
    let result = null;
    const dlg = modal(`Inherit ${fullName}`,
      [
        h('p', { class: 'muted' }, intro || 'The original is cloned and kept read-only. All work happens in a separate successor workspace, and nothing is pushed anywhere.'),
        h('fieldset', { class: 'mode-options' }, h('legend', null, 'Where may inherited code run?'), options),
        warnBox,
      ],
      [cancel, confirm],
      { onClose: () => resolve(result) });
    cancel.addEventListener('click', () => dlg.close());
    confirm.addEventListener('click', () => { result = chosen; dlg.close(); });
  });
}

// ---------- File viewer ----------

/** Open a source file in a modal, highlighting [start, end]. */
export function openFileViewer(id, path, start, end, side = 'successor') {
  const body = h('div', { class: 'file-viewer' }, spinner('Loading file'));
  const sideSel = h('select', { 'aria-label': 'Which copy to view' },
    h('option', { value: 'successor', selected: side === 'successor' }, 'Successor'),
    h('option', { value: 'original', selected: side === 'original' }, 'Original (read-only)'));
  const meta = h('span', { class: 'muted mono small' });
  modal(h('span', { class: 'mono' }, path), body, [meta, h('span', { class: 'spacer' }), h('label', { class: 'inline-label' }, 'Copy ', sideSel)], { wide: true });

  async function load(which) {
    replace(body, spinner('Loading file'));
    try {
      const f = await api.file(id, path, which);
      const lines = String(f.content ?? '').split('\n');
      const table = h('div', { class: 'code-lines', role: 'region', 'aria-label': `${path} source` });
      let firstHl = null;
      lines.forEach((text, idx) => {
        const n = idx + 1;
        const hl = start && n >= start && n <= (end || start);
        const row = h('div', { class: ['code-line', hl && 'hl'] },
          h('span', { class: 'ln', 'aria-hidden': 'true' }, String(n)),
          h('span', { class: 'lc' }, text || ' '));
        if (hl && !firstHl) firstHl = row;
        table.append(row);
      });
      replace(body, table);
      replace(meta, `${fmtNum(f.lines ?? lines.length)} lines${f.truncated ? ' · truncated' : ''}${start ? ` · lines ${start}–${end || start} highlighted` : ''}`);
      if (firstHl) requestAnimationFrame(() => firstHl.scrollIntoView({ block: 'center' }));
    } catch (e) {
      replace(body, errorBox(e.message));
    }
  }
  sideSel.addEventListener('change', () => load(sideSel.value));
  load(side);
}

// ---------- Evidence & claims ----------

export function evidenceItem(ev, ctx) {
  const range = ev.start_line ? `:${ev.start_line}${ev.end_line && ev.end_line !== ev.start_line ? `–${ev.end_line}` : ''}` : '';
  const check = ev.check || 'unchecked';
  return h('li', { class: 'evidence' },
    h('div', { class: 'evidence-head' },
      h('button', { class: 'link mono', type: 'button', onclick: () => openFileViewer(ctx.id, ev.path, ev.start_line, ev.end_line) }, `${ev.path}${range}`),
      h('span', { class: `check tone-${tone(check)}`, title: CHECK_LABEL[check] || check },
        h('span', { 'aria-hidden': 'true' }, CHECK_ICON[check] || '·'), ' ', CHECK_LABEL[check] || titleCase(check))),
    ev.quote ? h('pre', { class: 'quote' }, ev.quote) : null);
}

export function evidenceList(evidence, ctx, key, { openByDefault = false } = {}) {
  if (!evidence || !evidence.length) return h('p', { class: 'muted small' }, 'No evidence cited.');
  const verified = evidence.filter((e) => e.check === 'verified').length;
  return details(key, ctx.openSet,
    h('span', null, `Evidence (${evidence.length})`, h('span', { class: 'muted small' }, ` · ${verified} verified against source`)),
    h('ul', { class: 'evidence-list' }, evidence.map((e) => evidenceItem(e, ctx))),
    { openByDefault, className: 'evidence-details' });
}

export function claimCard(claim, ctx, { compact = false } = {}) {
  if (!claim) return null;
  return h('article', { class: 'claim', id: claim.id ? `claim-${claim.id}` : undefined },
    h('div', { class: 'claim-head' },
      pill(claim.status || 'unknown'),
      claim.agent ? h('span', { class: 'agent-tag' }, titleCase(claim.agent)) : null,
      claim.id ? h('span', { class: 'muted mono small' }, claim.id) : null),
    h('p', { class: 'claim-text' }, claim.text || '—'),
    compact ? null : evidenceList(claim.evidence, ctx, `ev-${claim.id || claim.text}`));
}

// ---------- Lifecycle ----------

export function lifecycle(stage) {
  const idx = STAGES.indexOf(stage);
  return h('ol', { class: 'lifecycle', 'aria-label': 'Lifecycle stage' },
    STAGES.map((s, i) => h('li', {
      class: ['lifecycle-step', i < idx && 'done', i === idx && 'current'],
      'aria-current': i === idx ? 'step' : undefined,
    }, h('span', { class: 'lifecycle-dot', 'aria-hidden': 'true' }), h('span', { class: 'lifecycle-label' }, titleCase(s)))));
}

export function lifecycleBar(stage) {
  const idx = Math.max(0, STAGES.indexOf(stage));
  return h('div', { class: 'lifecycle-bar', role: 'img', 'aria-label': `Stage ${idx + 1} of ${STAGES.length}: ${titleCase(stage)}` },
    STAGES.map((s, i) => h('span', { class: ['seg', i <= idx && 'on'] })));
}

// ---------- Verification ----------

export function verificationSummary(v) {
  if (!v) return 'Not verified yet';
  if (v.status === 'not_run') return `Not run${v.reason ? ` — ${v.reason}` : ''}`;
  const s = v.summary;
  if (s) {
    const parts = [`${fmtNum(s.passed)} passed`];
    if (s.failed) parts.push(`${fmtNum(s.failed)} failed`);
    if (s.errors) parts.push(`${fmtNum(s.errors)} errors`);
    if (s.skipped) parts.push(`${fmtNum(s.skipped)} skipped`);
    return parts.join(' · ');
  }
  return `exit code ${v.exit_code ?? '—'}`;
}

export function verificationBadge(v) {
  if (!v) return pill('not_run', 'Not verified');
  return h('span', { class: 'ver-badge' }, pill(v.status), h('span', { class: 'small muted' }, ` ${verificationSummary(v)}`));
}

export { fmtDate, shortSha };
