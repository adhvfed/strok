export const MIN_ZOOM = 0.1;
export const MAX_ZOOM = 64;

const copyBox = (box) => ({
  x: Number(box.x),
  y: Number(box.y),
  width: Number(box.width),
  height: Number(box.height),
});

const clamp = (value, minimum, maximum) => Math.max(minimum, Math.min(maximum, value));

/** Counter-transform a marker around its center so it stays screen-aligned. */
export function screenAlignedMarkerTransform(matrix, x, y, pixels) {
  const determinant = matrix.a * matrix.d - matrix.b * matrix.c;
  if (![matrix.a, matrix.b, matrix.c, matrix.d, x, y, pixels].every(Number.isFinite)
    || Math.abs(determinant) < 1e-12 || pixels <= 0) return null;
  const a = matrix.d / determinant * pixels;
  const b = -matrix.b / determinant * pixels;
  const c = -matrix.c / determinant * pixels;
  const d = matrix.a / determinant * pixels;
  const e = x - a * x - c * y;
  const f = y - b * x - d * y;
  return [a, b, c, d, e, f].map((value) => Object.is(value, -0) ? 0 : value);
}

/**
 * View-only SVG camera state. It deliberately knows nothing about the DOM,
 * document edits, or history; callers translate screen coordinates into world
 * coordinates and apply `viewBox` to their renderer.
 */
export class ViewBoxCamera {
  constructor({ minZoom = MIN_ZOOM, maxZoom = MAX_ZOOM } = {}) {
    this.minZoom = minZoom;
    this.maxZoom = maxZoom;
    this.id = null;
    this.fitBox = null;
    this.viewBox = null;
  }

  get ready() {
    return this.fitBox !== null && this.viewBox !== null;
  }

  get zoom() {
    return this.ready ? this.fitBox.width / this.viewBox.width : 1;
  }

  /** Install new content, preserving the view only when its identity matches. */
  install(id, fit) {
    const nextFit = copyBox(fit);
    if (![nextFit.x, nextFit.y, nextFit.width, nextFit.height].every(Number.isFinite)
      || nextFit.width <= 0 || nextFit.height <= 0) return false;

    const sameView = this.id === id && this.viewBox !== null;
    const zoom = sameView ? this.zoom : 1;
    const center = sameView
      ? {
          x: this.viewBox.x + this.viewBox.width / 2,
          y: this.viewBox.y + this.viewBox.height / 2,
        }
      : {
          x: nextFit.x + nextFit.width / 2,
          y: nextFit.y + nextFit.height / 2,
        };
    this.id = id;
    this.fitBox = nextFit;
    this.viewBox = {
      x: center.x - nextFit.width / zoom / 2,
      y: center.y - nextFit.height / zoom / 2,
      width: nextFit.width / zoom,
      height: nextFit.height / zoom,
    };
    return true;
  }

  fit() {
    if (!this.fitBox) return false;
    this.viewBox = copyBox(this.fitBox);
    return true;
  }

  /** Zoom while keeping the supplied world/fraction anchor fixed. */
  zoomBy(factor, anchor = null) {
    if (!this.ready || !Number.isFinite(factor) || factor <= 0) return false;
    const before = this.zoom;
    const after = clamp(before * factor, this.minZoom, this.maxZoom);
    if (Math.abs(after - before) < 1e-9) return false;

    const fixed = anchor ?? {
      x: this.viewBox.x + this.viewBox.width / 2,
      y: this.viewBox.y + this.viewBox.height / 2,
      fractionX: 0.5,
      fractionY: 0.5,
    };
    const width = this.fitBox.width / after;
    const height = this.fitBox.height / after;
    this.viewBox = {
      x: fixed.x - fixed.fractionX * width,
      y: fixed.y - fixed.fractionY * height,
      width,
      height,
    };
    return true;
  }

  panByWorld(dx, dy) {
    if (!this.viewBox || !Number.isFinite(dx) || !Number.isFinite(dy)) return false;
    this.viewBox.x += dx;
    this.viewBox.y += dy;
    return true;
  }
}
