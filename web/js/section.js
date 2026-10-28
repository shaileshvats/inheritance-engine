// A region of the page that re-renders only when its data changes, and never
// while the user is typing in it or has explicitly locked it (e.g. an open editor).
import { replace, sig } from './dom.js';

function isEditing(el) {
  const a = document.activeElement;
  if (!a || !el.contains(a)) return false;
  return a.matches('input, textarea, select, [contenteditable="true"]');
}

export class Section {
  constructor(el, render) {
    this.el = el;
    this.render = render;
    this.sig = undefined;
    this.locked = false;
  }

  /** Re-render if `key` (or the data's signature) changed. */
  update(data, key) {
    const s = key ?? sig(data);
    if (s === this.sig || this.locked || isEditing(this.el)) return false;
    this.sig = s;
    replace(this.el, this.render(data));
    return true;
  }

  /** Force the next update to re-render. */
  invalidate() { this.sig = undefined; }
  lock() { this.locked = true; }
  unlock() { this.locked = false; this.sig = undefined; }
}
