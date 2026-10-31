// Overview: Lead Engineer chat (primary interaction) and key facts.
import { h, replace, pill, markdown, spinner, errorBox, fmtDate, fmtNum, sig, titleCase, emptyState } from '../dom.js';
import { api, runSuggestedAction, ACTION_LABELS } from '../api.js';
import { claimCard, verificationBadge } from '../components.js';
import { Section } from '../section.js';

export const PRESETS = [
  'Understand this project. Tell me what its creator was trying to build, how it works, and which components are essential.',
  'Find what is broken and fix the important problems without destroying the original behaviour.',
];

export class OverviewTab {
  constructor(ctx) {
    this.ctx = ctx;
    this.messages = [];
    this.pending = null;
    this.jobSig = null;
  }

  mount() {
    this.logEl = h('div', { class: 'chat-log', role: 'log', 'aria-live': 'polite', 'aria-label': 'Conversation with the Lead Engineer' });
    this.input = h('textarea', { class: 'input chat-input', rows: 3, id: 'chat-input', placeholder: 'Ask the Lead Engineer…', 'aria-label': 'Message to the Lead Engineer' });
    this.sendBtn = h('button', { class: 'btn btn-primary', type: 'submit' }, 'Send');
    const form = h('form', { class: 'chat-form' }, this.input,
      h('div', { class: 'chat-form-row' },
        h('div', { class: 'presets' }, PRESETS.map((p, i) => h('button', {
          type: 'button', class: 'chip-btn', title: p,
          onclick: () => this.send(p),
        }, i === 0 ? 'Understand this project' : 'Find & fix what is broken'))),
        this.sendBtn));
    form.addEventListener('submit', (e) => { e.preventDefault(); this.send(this.input.value); });
    this.input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); form.requestSubmit(); }
    });

    this.factsEl = h('aside', { class: 'facts-panel', 'aria-label': 'Key facts' });
    this.facts = new Section(this.factsEl, (d) => this.renderFacts(d));

    this.loadChat();
    return h('div', { class: 'overview-grid' },
      h('section', { class: 'chat card', 'aria-labelledby': 'chat-title' },
        h('div', { class: 'chat-head' },
          h('h2', { id: 'chat-title' }, 'Lead Engineer'),
          h('p', { class: 'small muted' }, 'Answers are grounded in this inheritance\'s findings. Suggested actions never run until you press them.')),
        this.logEl, form),
      this.factsEl);
  }

  update(d) {
    this.facts.update(d, sig([d.report && d.report.purpose, d.report && d.report.next_actions, d.last_verification, d.issues, d.counts, d.stage, d.upstream_triage, !!d.active_job]));
    // Refresh the conversation whenever a job changes state (the lead may have posted a summary).
    const js = sig((d.jobs || []).map((j) => [j.id, j.status]));
    if (this.jobSig !== null && js !== this.jobSig && !this.pending) this.loadChat();
    this.jobSig = js;
  }

  async loadChat() {
    try {
      this.messages = await api.chat(this.ctx.id);
      this.renderLog();
    } catch (e) {
      replace(this.logEl, errorBox(e.message));
    }
  }

  renderLog() {
    const items = this.messages.map((m) => this.message(m));
    if (this.pending) {
      items.push(this.message({ role: 'user', content: this.pending.text, at: this.pending.at }));
      items.push(h('div', { class: 'msg msg-lead msg-pending' },
        h('div', { class: 'msg-meta' }, h('span', { class: 'agent-tag' }, 'Lead · Gemma')),
        h('div', { class: 'msg-body' }, spinner('Thinking'), ' Thinking locally — this can take a minute on a laptop…')));
    }
    if (!items.length) {
      replace(this.logEl, emptyState('No conversation yet.',
        'Ask the Lead Engineer anything about this project, or start with one of the suggested prompts below.'));
      return;
    }
    replace(this.logEl, items);
    this.logEl.scrollTop = this.logEl.scrollHeight;
  }

  message(m) {
    const isLead = m.role === 'lead';
    const action = isLead && m.suggested_action && m.suggested_action !== 'none' ? m.suggested_action : null;
    let actionBtn = null;
    if (action) {
      actionBtn = h('button', { class: 'btn btn-primary btn-sm', type: 'button' }, ACTION_LABELS[action] || titleCase(action));
      actionBtn.addEventListener('click', async () => {
        if (this.ctx.jobActive()) { actionBtn.replaceWith(h('span', { class: 'small muted' }, 'A job is already running. Wait for it to finish first.')); return; }
        actionBtn.disabled = true;
        const job = await this.ctx.startJob(runSuggestedAction(this.ctx.id, action));
        actionBtn.textContent = job ? 'Started ✓' : 'Could not start';
      });
    }
    return h('div', { class: ['msg', isLead ? 'msg-lead' : 'msg-user'] },
      h('div', { class: 'msg-meta' },
        isLead ? h('span', { class: 'agent-tag', title: m.via === 'mastra' ? 'Answered by the Mastra Lead Engineer (companion/mastra) over Gemma' : 'Answered by the engine’s built-in Lead over Gemma' }, m.via === 'mastra' ? 'Lead · Mastra · Gemma' : 'Lead · Gemma') : h('span', { class: 'you-tag' }, 'You'),
        h('time', { class: 'small muted', datetime: m.at }, fmtDate(m.at, true)),
        isLead && m.call_id ? h('button', { class: 'link small', type: 'button', onclick: () => this.ctx.goTab('agents', { call: m.call_id }) }, 'trace') : null),
      h('div', { class: 'msg-body' }, isLead ? markdown(m.content) : h('p', null, m.content)),
      actionBtn ? h('div', { class: 'msg-action' }, h('span', { class: 'small muted' }, 'Suggested next step:'), actionBtn) : null);
  }

  async send(text) {
    const msg = String(text || '').trim();
    if (!msg || this.pending) return;
    this.pending = { text: msg, at: new Date().toISOString() };
    this.input.value = '';
    this.sendBtn.disabled = true;
    this.renderLog();
    try {
      await api.sendChat(this.ctx.id, msg);
      this.pending = null;
      await this.loadChat();
    } catch (e) {
      this.pending = null;
      this.input.value = msg;
      this.renderLog();
      this.logEl.append(errorBox(e.message));
    } finally {
      this.sendBtn.disabled = false;
    }
  }

  /** Ask the lead to explain issues (used by the Restoration tab). */
  focus(f) { if (f && f.prompt) this.send(f.prompt); else if (f && f.input) this.input.focus(); }

  /** Upstream issues of the original repository, triaged against the successor by the Mastra workflow. */
  triageCard(d) {
    const t = d.upstream_triage;
    const VERDICT = { addressed_by_successor: ['passed', 'Addressed'], still_relevant: ['info', 'Still relevant'], obsolete: ['muted', 'Obsolete'], needs_human: ['warn', 'Needs you'] };
    const start = h('button', { class: 'btn btn-secondary btn-sm', type: 'button', disabled: !!d.active_job }, t ? 'Re-run triage' : 'Triage upstream issues');
    const note = h('p', { class: 'small muted', role: 'status', 'aria-live': 'polite' });
    start.addEventListener('click', async () => {
      start.disabled = true;
      const job = await this.ctx.startJob(api.triage(this.ctx.id));
      note.textContent = job ? 'Triage started: the Mastra workflow classifies each open issue with Gemma.' : 'Could not start triage (is the Mastra companion running?).';
    });
    return h('div', { class: 'card facts-card' },
      h('h2', null, 'Upstream issues'),
      h('p', { class: 'small muted' }, 'Open issues on the original repository, classified against the successor’s changes by a Mastra workflow. "Addressed" is kept only when it cites a real repair or commit.'),
      t ? [
        h('p', { class: 'small' }, `${fmtNum(t.total_open_upstream)} open upstream · triaged ${fmtNum((t.triaged || []).length)} · ${fmtDate(t.triaged_at, true)}`),
        h('ul', { class: 'triage-list' }, (t.triaged || []).map((x) => {
          const [state, label] = VERDICT[x.verdict] || ['muted', titleCase(x.verdict)];
          return h('li', null,
            pill(state, label), ' ',
            h('a', { href: x.url, target: '_blank', rel: 'noopener noreferrer' }, `#${x.number}`), ' ', x.title,
            h('details', { class: 'small' }, h('summary', null, 'why'), h('p', null, x.reason), h('p', { class: 'muted' }, `Engine check: ${x.engine_check}`)));
        })),
      ] : null,
      h('div', { class: 'row' }, start),
      note);
  }

  /** Live web context (SerpApi), summarised by Gemma; every item cites a search result the engine checked. */
  webCard(d) {
    const w = d.web_research;
    if (!w && !this.ctx.status?.web_search?.enabled) return null;
    const start = h('button', { class: 'btn btn-secondary btn-sm', type: 'button', disabled: !!d.active_job }, w ? 'Search again' : 'Research on the web');
    const note = h('p', { class: 'small muted', role: 'status', 'aria-live': 'polite' });
    start.addEventListener('click', async () => {
      start.disabled = true;
      const job = await this.ctx.startJob(api.research(this.ctx.id));
      note.textContent = job ? 'Searching the web (3 searches), then Gemma summarises with citations.' : 'Could not start web research.';
    });
    const link = (url, text) => h('a', { href: url, target: '_blank', rel: 'noopener noreferrer' }, text);
    const byId = Object.fromEntries((w?.results || []).map((r) => [r.id, r]));
    const cites = (ids) => h('span', { class: 'small muted' }, ' ', (ids || []).map((c, i) => [i ? ' ' : '', byId[c] ? link(byId[c].url, `[${c}]`) : `[${c}]`]));
    return h('div', { class: 'card facts-card' },
      h('h2', null, 'On the web'),
      h('p', { class: 'small muted' }, 'Live search results (SerpApi) about this project, summarised by Gemma. Every item must cite a real result, or the engine drops it.'),
      w ? [
        h('p', { class: 'small' }, w.summary, cites(w.summary_cites)),
        (w.related || []).length ? h('ul', { class: 'triage-list' }, w.related.map((r) => h('li', null, pill('info', titleCase(r.relation)), ' ', link(r.url, r.name), ' ', h('span', { class: 'small muted' }, r.note), cites(r.cites),
          r.engine_note ? h('span', { class: 'small engine-note' }, ` Engine: ${r.engine_note}.`) : null))) : null,
        (w.documentation || []).length ? h('ul', { class: 'triage-list' }, w.documentation.map((x) => h('li', null, pill('muted', 'Docs'), ' ', link(x.url, x.title), cites(x.cites)))) : null,
        h('details', { class: 'small' }, h('summary', null, `${fmtNum((w.results || []).length)} results from ${fmtNum((w.queries || []).length)} searches · ${fmtNum(w.dropped_uncited || 0)} uncited item(s) dropped · ${fmtDate(w.researched_at, true)}`),
          h('ol', null, (w.results || []).map((r) => h('li', null, `[${r.id}] `, link(r.url, r.title || r.url), ' ', h('span', { class: 'muted' }, r.source))))),
      ] : null,
      h('div', { class: 'row' }, start),
      note);
  }

  renderFacts(d) {
    const issues = d.issues || [];
    const open = issues.filter((i) => i.status === 'open');
    const essential = open.filter((i) => i.category === 'essential');
    const r = d.report;
    return [
      h('div', { class: 'card facts-card' },
        h('h2', null, 'What it is'),
        r && r.purpose ? claimCard(r.purpose, this.ctx, { compact: true })
          : h('p', { class: 'muted' }, d.active_job ? 'The agent team is still investigating…' : 'No purpose established yet. Run an investigation to find out.'),
        r && r.purpose ? h('button', { class: 'link small', type: 'button', onclick: () => this.ctx.goTab('report') }, 'See evidence and full report →') : null),
      h('div', { class: 'card facts-card' },
        h('h2', null, 'Health'),
        verificationBadge(d.last_verification),
        d.last_verification && d.last_verification.command ? h('p', { class: 'small muted' }, h('code', null, d.last_verification.command)) : null,
        h('dl', { class: 'mini-stats' },
          stat('Open issues', fmtNum(open.length)),
          stat('Essential', fmtNum(essential.length)),
          stat('Repairs', fmtNum((d.repairs || []).length)),
          stat('Agent calls', fmtNum(d.counts?.agent_calls ?? 0))),
        essential.length ? h('ul', { class: 'issue-mini' }, essential.slice(0, 4).map((i) => h('li', null, pill(i.severity), ' ', i.title))) : null,
        open.length ? h('button', { class: 'link small', type: 'button', onclick: () => this.ctx.goTab('restoration') }, 'Go to restoration →') : null),
      this.triageCard(d),
      this.webCard(d),
      r && (r.next_actions || []).length ? h('div', { class: 'card facts-card' },
        h('h2', null, 'Next actions'),
        h('p', { class: 'small muted' }, 'Suggested by the Lead Engineer'),
        h('ol', { class: 'next-actions' }, r.next_actions.map((a) => h('li', null, a)))) : null,
      h('p', { class: 'small muted facts-foot' }, `Stage: ${titleCase(d.stage)} · inherited ${fmtDate(d.created_at)}`),
    ];
  }
}

function stat(label, value) {
  return h('div', null, h('dt', null, label), h('dd', null, value));
}
