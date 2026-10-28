// Entry point: theme, system status chips and hash routing.
import { h, replace } from './js/dom.js';
import { api, Poller } from './js/api.js';
import { DashboardView } from './js/views/dashboard.js';
import { DetailView } from './js/views/detail.js';

// ---------- Theme ----------

function effectiveTheme() {
  const t = document.documentElement.dataset.theme;
  if (t === 'light' || t === 'dark') return t;
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

function setupTheme() {
  const btn = document.getElementById('theme-toggle');
  const label = () => btn.setAttribute('aria-label', effectiveTheme() === 'dark' ? 'Switch to light theme' : 'Switch to dark theme');
  label();
  btn.addEventListener('click', () => {
    const next = effectiveTheme() === 'dark' ? 'light' : 'dark';
    document.documentElement.dataset.theme = next;
    try { localStorage.setItem('ie-theme', next); } catch { /* storage unavailable */ }
    label();
  });
}

// ---------- Status chips ----------

function chip(toneName, label, title) {
  return h('span', { class: `status-chip tone-${toneName}`, title },
    h('span', { class: 'dot', 'aria-hidden': 'true' }), label);
}

export let systemStatus = null;

function mastraChip(m) {
  if (!m || !m.configured) return null;
  if (!m.reachable) return chip('warn', 'Mastra offline', `Mastra companion not reachable at ${m.url}; the built-in Lead answers instead.`);
  return chip('good', `Mastra · ${m.tool_mode || '?'}`, `Mastra companion at ${m.url}: Lead Engineer agent and upstream-issue triage workflow. Tool mode: ${m.tool_mode} (${m.tool_mode === 'native' ? 'the model chooses tools' : 'a workflow calls the tools; this runtime cannot do tool calling'}).`);
}

async function refreshStatus() {
  const el = document.getElementById('status-chips');
  try {
    const st = await api.status();
    systemStatus = st;
    const m = st.model || {};
    const d = st.docker || {};
    replace(el,
      chip(m.reachable ? 'good' : 'bad',
        `${m.model || 'model'} · ${m.locality || '?'}`,
        m.reachable
          ? `Model reachable via ${m.provider} at ${m.endpoint}`
          : `Model not reachable${m.detail ? `: ${m.detail}` : ''}`),
      chip(d.available ? 'good' : 'warn',
        d.available ? `Docker ${d.version || ''}`.trim() : 'Docker unavailable',
        d.available ? 'Docker sandbox available for running inherited code' : (d.detail || 'Docker is not running; verification needs Docker or an explicit native choice')),
      chip(st.privacy_mode === 'local_only' ? 'good' : 'info',
        (st.privacy_mode || 'unknown').replace(/_/g, ' '),
        'Model privacy policy: where source code and conversations may be sent'),
      chip(st.github_token ? 'good' : 'muted',
        st.github_token ? 'GitHub token' : 'GitHub anonymous',
        st.github_token ? 'Using a GitHub token from the environment (read-only use)' : 'Anonymous GitHub API: low rate limits'),
      mastraChip(st.mastra),
      st.sentry && st.sentry.enabled ? chip('good', 'Sentry tracing', 'Agent traces are sent to Sentry: metadata only (roles, models, tokens, latency, outcomes); no prompts, responses or source code') : null);
  } catch (e) {
    systemStatus = null;
    replace(el, chip('bad', 'Server unreachable', e.message));
  }
}

// ---------- Routing ----------

let current = null;

function parse(hash) {
  const m = /^#\/i\/([^/]+)(?:\/([a-z]+))?\/?$/.exec(hash || '');
  if (m) return { kind: 'detail', id: decodeURIComponent(m[1]), tab: m[2] || 'overview' };
  return { kind: 'dashboard' };
}

function route() {
  const r = parse(location.hash);
  const main = document.getElementById('main');
  if (current && r.kind === 'detail' && current.kind === 'detail' && current.id === r.id) {
    current.view.setTab(r.tab);
    return;
  }
  if (current) current.view.destroy();
  replace(main);
  const view = r.kind === 'detail' ? new DetailView(r.id, r.tab) : new DashboardView();
  current = { ...r, view };
  main.append(view.mount());
  window.scrollTo(0, 0);
  const heading = main.querySelector('h1');
  if (heading) { heading.setAttribute('tabindex', '-1'); heading.focus({ preventScroll: true }); }
}

setupTheme();
new Poller(refreshStatus, () => 30000).start();
window.addEventListener('hashchange', route);
route();
