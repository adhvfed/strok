// Selection state for the point editor. This module deliberately knows
// nothing about SVG or the watch protocol, so interactions and rendering can
// share one stable source of truth.
export class PointSelection {
  #anchors = new Set();
  #primary = null;
  #control = null;

  clear() {
    this.#anchors.clear();
    this.#primary = null;
    this.#control = null;
  }

  replaceAnchor(name) {
    this.#anchors.clear();
    this.#anchors.add(name);
    this.#primary = name;
    this.#control = null;
  }

  toggleAnchor(name) {
    this.#control = null;
    if (this.#anchors.delete(name)) {
      if (this.#primary === name) {
        this.#primary = this.#anchors.values().next().value ?? null;
      }
      return false;
    }
    this.#anchors.add(name);
    this.#primary = name;
    return true;
  }

  focusAnchor(name) {
    if (!this.#anchors.has(name)) return false;
    this.#primary = name;
    this.#control = null;
    return true;
  }

  selectControl(point, handle, anchor) {
    this.#anchors.clear();
    this.#primary = anchor;
    this.#control = { point, handle, anchor };
  }

  selectAll(names) {
    this.#anchors = new Set(names);
    this.#primary = names.at(-1) ?? null;
    this.#control = null;
  }

  reconcile(names) {
    const valid = new Set(names);
    for (const name of this.#anchors) {
      if (!valid.has(name)) this.#anchors.delete(name);
    }
    if (this.#primary && !valid.has(this.#primary)) this.#primary = null;
    if (this.#control && !valid.has(this.#control.anchor)) this.#control = null;
    if (!this.#primary && this.#anchors.size) {
      this.#primary = this.#anchors.values().next().value;
    }
  }

  hasAnchor(name) {
    return this.#anchors.has(name);
  }

  anchorNames() {
    return [...this.#anchors];
  }

  get anchorCount() {
    return this.#anchors.size;
  }

  get primaryAnchor() {
    return this.#primary;
  }

  get control() {
    return this.#control;
  }
}
