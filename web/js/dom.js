// DOM helpers. Everything that reaches the page goes through text nodes or
// attribute setters, never innerHTML: repository and model output is untrusted.

const SVG_NS = 'http://www.w3.org/2000/svg';

function setAttrs(el, attrs) {
  if (!attrs) return;
  for (const [k, v] of Object.entries(attrs)) {
    if (v == null || v === false) continue;
    if (k === 'class') el.setAttribute('class', Array.isArray(v) ? v.filter(Boolean).join(' ') : v);
    else if (k === 'style' && typeof v === 'object') Object.assign(el.style, v);
    else if (k === 'dataset') Object.assign(el.dataset, v);
    else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2).toLowerCase(), v);
    else if (k === 'href' || k === 'src') el.setAttribute(k, safeUrl(v));
    else if (k === 'value' && 'value' in el) el.value = v;
    else if (k === 'checked' || k === 'disabled' || k === 'open' || k === 'selected') el[k] = !!v;
    else if (v === true) el.setAttribute(k, '');
    else el.setAttribute(k, String(v));
  }
}

function appendChildren(el, children) {
  for (const c of children.flat(Infinity)) {
    if (c == null || c === false || c === true) continue;
    el.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
}

/** Create an HTML element. String children become text nodes. */
export function h(tag, attrs, ...children) {
  const el = document.createElement(tag);
  setAttrs(el, attrs);
  appendChildren(el, children);
  return el;
}

/** Create an SVG element. */
export function s(tag, attrs, ...children) {
  const el = document.createElementNS(SVG_NS, tag);
  setAttrs(el, attrs);
  appendChildren(el, children);
  return el;
}

/** Only allow http(s), mailto, same-origin paths and hash links. */
export function safeUrl(url) {
  const str = String(url ?? '');
  if (/^(https?:|mailto:)/i.test(str)) return str;
  if (str.startsWith('/') || str.startsWith('#') || str.startsWith('./')) return str;
  return '#';
}

export function clear(el) { while (el.firstChild) el.removeChild(el.firstChild); return el; }

export function replace(el, ...children) { clear(el); appendChildren(el, children); return el; }

/** Announce a short message to screen readers. */
export function announce(text) {
  const el = document.getElementById('sr-announcer');
  if (!el) return;
  el.textContent = '';
  setTimeout(() => { el.textContent = text; }, 30);
}

// ---------- Formatting ----------

export function fmtDate(iso, withTime = false) {
  if (!iso) return '—';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return String(iso);
  const opts = withTime
    ? { year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }
    : { year: 'numeric', month: 'short', day: 'numeric' };
  return d.toLocaleString(undefined, opts);
}

export function relTime(iso) {
  if (!iso) return '—';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return String(iso);
  const sec = Math.round((Date.now() - d.getTime()) / 1000);
  const abs = Math.abs(sec);
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' });
  const units = [[60, 'second'], [3600, 'minute'], [86400, 'hour'], [2592000, 'day'], [31536000, 'month'], [Infinity, 'year']];
  const div = { second: 1, minute: 60, hour: 3600, day: 86400, month: 2592000, year: 31536000 };
  for (const [limit, unit] of units) {
    if (abs < limit) return rtf.format(-Math.round(sec / div[unit]), unit);
  }
  return fmtDate(iso);
}

export function fmtDuration(ms) {
  if (ms == null) return '—';
  if (ms < 1000) return `${ms} ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(s < 10 ? 1 : 0)} s`;
  const m = Math.floor(s / 60);
  return `${m} min ${Math.round(s % 60)} s`;
}

export function fmtNum(n) { return n == null ? '—' : Number(n).toLocaleString(); }

export function fmtBytes(n) {
  if (n == null) return '—';
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function shortSha(sha) { return sha ? String(sha).slice(0, 8) : '—'; }

export function plural(n, word, pl) { return `${fmtNum(n)} ${n === 1 ? word : (pl || word + 's')}`; }

export function titleCase(s) { return String(s ?? '').replace(/[_-]+/g, ' ').replace(/^\w/, (c) => c.toUpperCase()); }

// ---------- Small components ----------

const STATUS_TONE = {
  passed: 'good', verified: 'good', approve: 'good', applied: 'good', repaired: 'good', done: 'good', completed: 'good', agree: 'good', ok: 'good',
  failed: 'bad', error: 'bad', reject: 'bad', rejected: 'bad', disagree: 'bad', mismatch: 'bad', missing_file: 'bad', out_of_range: 'bad', high: 'bad',
  inferred: 'warn', request_changes: 'warn', reverted: 'warn', insufficient_evidence: 'warn', interrupted: 'warn', paused: 'warn', medium: 'warn', cancelled: 'warn',
  running: 'info', queued: 'info', open: 'info', investigating: 'info',
  unknown: 'muted', not_run: 'muted', unchecked: 'muted', skipped: 'muted', pending: 'muted', low: 'muted', none: 'muted',
};

export function tone(status) { return STATUS_TONE[status] || 'muted'; }

/** A status pill. `label` defaults to a humanised status. */
export function pill(status, label, extraClass) {
  return h('span', { class: ['pill', `tone-${tone(status)}`, extraClass] }, label ?? titleCase(status ?? 'unknown'));
}

export function copyButton(text, label = 'Copy') {
  const btn = h('button', { class: 'btn btn-ghost btn-xs', type: 'button', 'aria-label': `${label}: ${text}` }, label);
  btn.addEventListener('click', async () => {
    try { await navigator.clipboard.writeText(text); btn.textContent = 'Copied'; }
    catch { btn.textContent = 'Copy failed'; }
    setTimeout(() => { btn.textContent = label; }, 1400);
  });
  return btn;
}

export function emptyState(title, body, ...actions) {
  return h('div', { class: 'empty' },
    h('div', { class: 'empty-glyph', 'aria-hidden': 'true' }, '∅'),
    h('p', { class: 'empty-title' }, title),
    body ? h('p', { class: 'empty-body' }, body) : null,
    actions.length ? h('div', { class: 'empty-actions' }, actions) : null);
}

export function spinner(label = 'Loading') {
  return h('span', { class: 'spinner', role: 'status', 'aria-label': label });
}

export function errorBox(message) {
  return h('div', { class: 'error-box', role: 'alert' }, h('strong', null, 'Something went wrong. '), String(message));
}

/**
 * <details> whose open state survives re-renders. `store` is a Set of keys
 * held by the owning view.
 */
export function details(key, store, summary, body, { openByDefault = false, className } = {}) {
  const isOpen = store ? (store.has(key) || (openByDefault && !store.has(`closed:${key}`))) : openByDefault;
  const el = h('details', { class: className, open: isOpen }, h('summary', null, summary), body);
  if (store) {
    el.addEventListener('toggle', () => {
      if (el.open) { store.add(key); store.delete(`closed:${key}`); }
      else { store.delete(key); store.add(`closed:${key}`); }
    });
  }
  return el;
}

// ---------- Safe markdown subset ----------

function inline(text) {
  // Tokenise `code` and **bold**; everything becomes text nodes.
  const out = [];
  const re = /(`[^`\n]+`|\*\*[^*\n]+\*\*)/g;
  let last = 0; let m;
  while ((m = re.exec(text))) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const tok = m[0];
    if (tok.startsWith('`')) out.push(h('code', null, tok.slice(1, -1)));
    else out.push(h('strong', null, tok.slice(2, -2)));
    last = m.index + tok.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** Render a tiny markdown subset (headings, paragraphs, bullet/numbered lists, code fences, `code`, **bold**). */
export function markdown(src) {
  const root = h('div', { class: 'md' });
  const lines = String(src ?? '').replace(/\r\n/g, '\n').split('\n');
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) { i++; continue; }
    if (line.trim().startsWith('```')) {
      const buf = []; i++;
      while (i < lines.length && !lines[i].trim().startsWith('```')) buf.push(lines[i++]);
      i++;
      root.append(h('pre', { class: 'code-block' }, buf.join('\n')));
      continue;
    }
    const hm = /^(#{1,4})\s+(.*)$/.exec(line);
    if (hm) { root.append(h(hm[1].length <= 2 ? 'h4' : 'h5', null, inline(hm[2]))); i++; continue; }
    if (/^\s*([-*•]|\d+[.)])\s+/.test(line)) {
      const ordered = /^\s*\d+[.)]\s+/.test(line);
      const list = h(ordered ? 'ol' : 'ul');
      while (i < lines.length && /^\s*([-*•]|\d+[.)])\s+/.test(lines[i])) {
        list.append(h('li', null, inline(lines[i].replace(/^\s*([-*•]|\d+[.)])\s+/, ''))));
        i++;
      }
      root.append(list);
      continue;
    }
    const buf = [];
    while (i < lines.length && lines[i].trim() && !/^\s*([-*•]|\d+[.)])\s+/.test(lines[i]) && !/^#{1,4}\s/.test(lines[i]) && !lines[i].trim().startsWith('```')) buf.push(lines[i++]);
    root.append(h('p', null, inline(buf.join(' '))));
  }
  return root;
}

/** Render a unified diff with per-line colouring. */
export function diffView(diff) {
  const pre = h('div', { class: 'diff', role: 'region', 'aria-label': 'Unified diff' });
  if (!diff || !String(diff).trim()) { pre.append(h('div', { class: 'diff-line diff-meta' }, '(no changes)')); return pre; }
  for (const line of String(diff).split('\n')) {
    let cls = 'diff-ctx';
    if (line.startsWith('+++') || line.startsWith('---') || line.startsWith('diff ') || line.startsWith('index ')) cls = 'diff-meta';
    else if (line.startsWith('@@')) cls = 'diff-hunk';
    else if (line.startsWith('+')) cls = 'diff-add';
    else if (line.startsWith('-')) cls = 'diff-del';
    pre.append(h('div', { class: `diff-line ${cls}` }, line || ' '));
  }
  return pre;
}

/** Stable signature for change detection. */
export function sig(value) {
  try { return JSON.stringify(value); } catch { return String(Math.random()); }
}

export function storage(key, fallback) {
  try { const v = localStorage.getItem(key); return v == null ? fallback : JSON.parse(v); } catch { return fallback; }
}

export function store(key, value) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* storage unavailable */ }
}
