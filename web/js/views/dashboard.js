// Dashboard: Hunter search plus the user's current inheritances.
import { h, replace, pill, fmtDate, fmtNum, relTime, spinner, errorBox, emptyState, sig, storage, store, titleCase, announce } from '../dom.js';
import { api, Poller, ACTIVE_JOB } from '../api.js';
import { chooseExecutionMode, lifecycleBar, verificationBadge } from '../components.js';

const SIGNAL_KINDS = [
  ['historical', 'Historical value'],
  ['present', 'Present usefulness'],
  ['evolution', 'Evolution potential'],
  ['caution', 'Caution'],
];

const EXAMPLES = {
  problem: ['parse and validate ini config files', 'tiny static site generator', 'convert markdown tables to csv'],
  curiosity: ['retro game engine', 'forgotten python utilities', 'esoteric programming language interpreter'],
};

export class DashboardView {
  constructor() {
    this.mode = storage('ie-hunt-mode', 'problem');
    this.listSig = null;
    this.poller = null;
  }

  mount() {
    this.root = h('div', { class: 'page dashboard' });
    this.resultsEl = h('section', { class: 'hunt-results', 'aria-live': 'polite', 'aria-busy': 'false' });
    this.listEl = h('div', { class: 'inheritance-grid' }, spinner('Loading inheritances'));

    this.root.append(
      h('section', { class: 'hero' },
        h('h1', null, 'Hunter'),
        h('p', { class: 'lede' }, 'Search GitHub for software worth inheriting. Describe a problem you need solved, or wander through quiet corners out of curiosity. Results come straight from the GitHub API; the signals beside each project are evidence, not a score.'),
        this.searchForm()),
      this.resultsEl,
      h('section', { class: 'section' },
        h('div', { class: 'section-head' }, h('h2', null, 'Your inheritances')),
        this.listEl));

    this.poller = new Poller(() => this.loadList(), () => this.activeCount ? 2500 : 10000).start();
    return this.root;
  }

  destroy() { if (this.poller) this.poller.stop(); }

  // ---------- Search ----------

  searchForm() {
    const input = h('input', { type: 'search', id: 'hunt-q', class: 'input input-lg', autocomplete: 'off', required: true, placeholder: '' });
    const lang = h('input', { type: 'text', id: 'hunt-lang', class: 'input', placeholder: 'Any', autocomplete: 'off', size: 10 });
    const examples = h('div', { class: 'examples', 'aria-label': 'Example searches' });
    // Web search (SerpApi) is offered only when the engine has a key; the checkbox appears once status says so.
    const web = h('input', { type: 'checkbox', id: 'hunt-web' });
    const webField = h('label', { class: 'small web-field', hidden: true }, web, ' Also search the web (SerpApi)');
    api.status().then((st) => { if (st.web_search && st.web_search.enabled) webField.hidden = false; }).catch(() => {});
    const hint = h('p', { class: 'small muted', id: 'hunt-hint' });

    const setPlaceholder = () => {
      input.placeholder = this.mode === 'problem'
        ? 'Describe a problem, e.g. "I need to diff two CSV files from the command line"'
        : 'A topic, technology or era, e.g. "pixel art tools" or "python 2 utilities"';
      replace(hint, this.mode === 'problem'
        ? 'Problem-first: Gemma turns your description into a GitHub search query. The generated query is shown with the results.'
        : 'Curiosity-first: biased towards older, quieter repositories. No model call is needed.');
      replace(examples, h('span', { class: 'small muted' }, 'Try:'), EXAMPLES[this.mode].map((ex) =>
        h('button', { type: 'button', class: 'chip-btn', onclick: () => { input.value = ex; form.requestSubmit(); } }, ex)));
    };

    const seg = h('div', { class: 'segmented', role: 'radiogroup', 'aria-label': 'Discovery mode' },
      [['problem', 'Problem-first'], ['curiosity', 'Curiosity-first']].map(([value, label]) => {
        const btn = h('button', { type: 'button', role: 'radio', 'aria-checked': String(this.mode === value), class: 'seg-btn', dataset: { value } }, label);
        btn.addEventListener('click', () => {
          this.mode = value; store('ie-hunt-mode', value);
          seg.querySelectorAll('.seg-btn').forEach((b) => b.setAttribute('aria-checked', String(b.dataset.value === value)));
          setPlaceholder();
        });
        return btn;
      }));
    seg.addEventListener('keydown', (e) => {
      if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
      const btns = [...seg.querySelectorAll('.seg-btn')];
      const next = btns.find((b) => b.getAttribute('aria-checked') !== 'true');
      if (next) { next.click(); next.focus(); }
    });

    const submit = h('button', { type: 'submit', class: 'btn btn-primary btn-lg' }, 'Search');
    const form = h('form', { class: 'hunt-form', role: 'search' },
      h('div', { class: 'hunt-row' },
        h('label', { class: 'sr-only', for: 'hunt-q' }, 'Search for repositories'),
        input,
        h('label', { class: 'lang-field' }, h('span', { class: 'small muted' }, 'Language'), lang),
        submit),
      h('div', { class: 'hunt-meta' }, seg, examples, webField),
      hint);
    form.addEventListener('submit', (e) => { e.preventDefault(); this.search(input.value.trim(), lang.value.trim(), submit, !webField.hidden && web.checked); });
    setPlaceholder();
    return form;
  }

  async search(q, lang, submit, web) {
    if (!q) return;
    submit.disabled = true;
    this.resultsEl.setAttribute('aria-busy', 'true');
    replace(this.resultsEl, h('div', { class: 'loading-row' }, spinner('Searching'),
      (this.mode === 'problem' ? ' Asking Gemma for a search query, then searching GitHub' : ' Searching GitHub') + (web ? ' and the web…' : '…')));
    try {
      const res = await api.hunt(q, this.mode, lang, web);
      this.renderResults(res);
      announce(`${res.results.length} repositories found`);
    } catch (e) {
      replace(this.resultsEl, errorBox(e.message));
    } finally {
      submit.disabled = false;
      this.resultsEl.setAttribute('aria-busy', 'false');
    }
  }

  renderResults(res) {
    const head = h('div', { class: 'results-head' },
      h('h2', null, 'Candidates'),
      h('p', { class: 'small muted' },
        `${fmtNum(res.total_count)} matches on GitHub · showing ${res.results.length} · query `,
        h('code', null, res.query_used)),
      res.ai_query && res.ai_query.used
        ? h('p', { class: 'ai-note small' },
          h('span', { class: 'agent-tag' }, 'Hunter · Gemma'),
          ' wrote this query',
          res.ai_query.reasoning ? `: ${res.ai_query.reasoning}` : '.')
        : null,
      res.web && res.web.used ? webNote(res.web) : null);
    if (!res.results.length) {
      replace(this.resultsEl, head, emptyState('No repositories matched.', 'Try broader words, drop the language filter, or switch discovery mode.'));
      return;
    }
    replace(this.resultsEl, head, h('div', { class: 'candidate-list' }, res.results.map((c) => this.candidateCard(c))));
  }

  candidateCard(c) {
    const explainSlot = h('div', { class: 'explain-slot', 'aria-live': 'polite' });
    const explainBtn = h('button', { type: 'button', class: 'btn btn-ghost' }, 'Why is this interesting?');
    explainBtn.addEventListener('click', () => this.explain(c, explainSlot, explainBtn));
    const inheritBtn = h('button', { type: 'button', class: 'btn btn-primary' }, 'Inherit');
    inheritBtn.addEventListener('click', () => this.inherit(c, inheritBtn));

    const grouped = SIGNAL_KINDS.map(([kind, label]) => {
      const list = (c.signals || []).filter((s) => s.kind === kind);
      if (!list.length) return null;
      return h('div', { class: `signal-group signal-${kind}` },
        h('span', { class: 'signal-kind' }, label),
        h('ul', { class: 'signal-list' }, list.map((s) => h('li', { class: 'signal', title: s.detail || '' },
          s.label, s.detail ? h('span', { class: 'signal-detail' }, ` — ${s.detail}`) : null))));
    });

    return h('article', { class: 'candidate card' },
      h('div', { class: 'candidate-head' },
        h('div', null,
          h('h3', { class: 'candidate-name' },
            h('a', { href: c.html_url, target: '_blank', rel: 'noopener noreferrer' }, c.full_name)),
          h('p', { class: 'candidate-desc' }, c.description || h('span', { class: 'muted' }, 'No description.'))),
        c.archived ? pill('cancelled', 'Archived') : null),
      c.web_source ? h('p', { class: 'small muted' }, 'Found on the web: ',
        h('a', { href: c.web_source.url, target: '_blank', rel: 'noopener noreferrer' }, c.web_source.title || c.web_source.url)) : null,
      h('dl', { class: 'facts' },
        fact('Language', c.language || '—'),
        fact('Stars', fmtNum(c.stars)),
        fact('Forks', fmtNum(c.forks)),
        fact('Licence', c.license || 'None declared'),
        fact('Created', fmtDate(c.created_at)),
        fact('Last push', `${fmtDate(c.pushed_at)}`),
        fact('Quiet for', c.idle_years != null ? `${c.idle_years.toFixed(1)} years` : '—'),
        fact('Size', c.size_kb != null ? `${fmtNum(c.size_kb)} KB` : '—')),
      (c.topics || []).length ? h('ul', { class: 'topics', 'aria-label': 'Topics' }, c.topics.map((t) => h('li', null, t))) : null,
      h('div', { class: 'signals' }, grouped),
      explainSlot,
      h('div', { class: 'card-actions' }, explainBtn, inheritBtn));
  }

  async explain(c, slot, btn) {
    btn.disabled = true;
    replace(slot, h('div', { class: 'loading-row' }, spinner('Gemma is reading'), ' Gemma is reading the README locally. On a laptop this can take up to a minute…'));
    try {
      const r = await api.explain(c.full_name);
      const x = r.explanation || {};
      replace(slot, h('div', { class: 'explanation' },
        h('div', { class: 'explanation-head' },
          h('span', { class: 'agent-tag' }, 'Hunter · Gemma'),
          h('span', { class: 'small muted' }, `read ${fmtNum(r.readme_chars)} characters of README`)),
        h('p', null, x.summary || '—'),
        h('dl', { class: 'explain-grid' },
          h('div', null, h('dt', null, 'Historical value'), h('dd', null, x.historical_value || '—')),
          h('div', null, h('dt', null, 'Present usefulness'), h('dd', null, x.present_usefulness || '—')),
          h('div', null, h('dt', null, 'Evolution potential'), h('dd', null, x.evolution_potential || '—'))),
        (x.caveats || []).length ? h('div', { class: 'caveats' }, h('strong', null, 'Caveats'), h('ul', null, x.caveats.map((cv) => h('li', null, cv)))) : null,
        h('p', { class: 'small muted' }, 'AI-written from the README only; not verified against the code. Agent call ', h('code', null, r.call_id || '—'))));
      btn.textContent = 'Explain again';
    } catch (e) {
      replace(slot, errorBox(e.message));
    } finally {
      btn.disabled = false;
    }
  }

  async inherit(c, btn) {
    const mode = await chooseExecutionMode(c.full_name, 'docker');
    if (!mode) return;
    btn.disabled = true;
    btn.textContent = 'Cloning…';
    try {
      const inh = await api.create(c.full_name, mode);
      location.hash = `#/i/${encodeURIComponent(inh.id)}`;
    } catch (e) {
      btn.disabled = false;
      btn.textContent = 'Inherit';
      btn.closest('.candidate').querySelector('.explain-slot').replaceChildren(errorBox(e.message));
    }
  }

  // ---------- Inheritance list ----------

  async loadList() {
    let list;
    try { list = await api.list(); }
    catch (e) { replace(this.listEl, errorBox(e.message)); this.listSig = null; return; }
    this.activeCount = list.filter((i) => i.active_job && ACTIVE_JOB.has(i.active_job.status)).length;
    const s = sig(list);
    if (s === this.listSig) return;
    this.listSig = s;
    if (!list.length) {
      replace(this.listEl, emptyState('Nothing inherited yet.',
        'Search above, pick a repository that deserves a future, and choose Inherit. The original stays untouched; your work happens in a successor workspace.'));
      return;
    }
    replace(this.listEl, list.map((i) => inheritanceCard(i)));
  }
}

function fact(label, value) {
  return h('div', { class: 'fact' }, h('dt', null, label), h('dd', null, value));
}

function inheritanceCard(i) {
  const job = i.active_job;
  return h('a', { class: 'inh-card card', href: `#/i/${encodeURIComponent(i.id)}` },
    h('div', { class: 'inh-card-head' },
      h('h3', null, i.name),
      pill(i.stage === 'verified' ? 'passed' : (i.stage === 'investigating' || i.stage === 'restoring' ? 'running' : 'open'), titleCase(i.stage))),
    h('p', { class: 'small muted mono' }, i.full_name),
    lifecycleBar(i.stage),
    h('div', { class: 'inh-card-row' }, verificationBadge(i.last_verification)),
    job ? h('div', { class: 'inh-card-row small' },
      pill(job.status), ' ', titleCase(job.kind), job.step ? ` · ${titleCase(job.step)}` : '') : null,
    h('div', { class: 'inh-card-foot small muted' },
      `${fmtNum(i.counts?.findings ?? 0)} findings · ${fmtNum(i.counts?.issues ?? 0)} issues · ${fmtNum(i.counts?.repairs ?? 0)} repairs`,
      h('span', { class: 'spacer' }),
      `updated ${relTime(i.updated_at)}`));
}

/** What the web search did, and the non-repository pages it found, each linked to its source. */
function webNote(w) {
  if (w.error) return h('p', { class: 'small muted' }, 'Web search failed: ', w.error);
  return h('details', { class: 'small web-note' },
    h('summary', null, `Web search (SerpApi) · query `, h('code', null, w.query), ` · ${fmtNum(w.results_seen)} results · ${fmtNum(w.repositories_added)} repositories added`),
    (w.pages || []).length ? h('ul', null, w.pages.map((p) => h('li', null,
      h('a', { href: p.url, target: '_blank', rel: 'noopener noreferrer' }, p.title || p.url), ' ',
      h('span', { class: 'muted' }, p.source || ''), p.snippet ? h('div', { class: 'muted' }, p.snippet) : null))) : h('p', { class: 'muted' }, 'No other pages.'));
}
