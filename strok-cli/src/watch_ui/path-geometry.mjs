/** Pure shape topology and path geometry for the watch editor. */

export function contours(shape) {
  const starts = shape.points
    .map((point, index) => point.start ? index : -1)
    .filter((index) => index >= 0);
  return starts.map((begin, index) => [begin, starts[index + 1] ?? shape.points.length]);
}

export function contourFor(shape, index) {
  return contours(shape).find(([begin, end]) => index >= begin && index < end);
}

export function nextIndex(shape, index) {
  const contour = contourFor(shape, index);
  if (!contour) return null;
  const [begin, end] = contour;
  return index + 1 < end ? index + 1 : (shape.closed ? begin : null);
}

export function previousIndex(shape, index) {
  const contour = contourFor(shape, index);
  if (!contour) return null;
  const [begin, end] = contour;
  return index > begin ? index - 1 : (shape.closed ? end - 1 : index);
}

export function pointAt(shape, index) {
  return index === null ? undefined : shape.points[index];
}

export function attachedAnchorIndex(shape, index, handle) {
  return handle === 'c1' ? previousIndex(shape, index) : index;
}

export function handlesAtAnchor(shape, anchorIndex) {
  const handles = [];
  const anchor = pointAt(shape, anchorIndex);
  if (anchor?.c2 && anchor.controlsEditable) {
    handles.push({ index: anchorIndex, handle: 'c2' });
  }
  const next = nextIndex(shape, anchorIndex);
  const outgoing = pointAt(shape, next);
  if (outgoing?.c1 && outgoing.controlsEditable) {
    handles.push({ index: next, handle: 'c1' });
  }
  return handles;
}

export function oppositeHandle(shape, index, handle) {
  const anchorIndex = attachedAnchorIndex(shape, index, handle);
  return handlesAtAnchor(shape, anchorIndex)
    .find((candidate) => candidate.index !== index || candidate.handle !== handle) ?? null;
}

export function isRetracted(shape, index, handle) {
  const point = pointAt(shape, index);
  const anchor = pointAt(shape, attachedAnchorIndex(shape, index, handle));
  if (!point?.[handle] || !anchor) return true;
  return Math.hypot(point[handle][0] - anchor.x, point[handle][1] - anchor.y) < 1e-7;
}

const lerp = (a, b, t) => a + (b - a) * t;

function smoothControls(shape, targetIndex) {
  const target = pointAt(shape, targetIndex);
  const tension = target.tension ?? 0;
  const previous = previousIndex(shape, targetIndex);
  const previousPrevious = previousIndex(shape, previous);
  const nextOfPrevious = nextIndex(shape, previous) ?? previous;
  const nextOfTarget = nextIndex(shape, targetIndex) ?? targetIndex;
  const point = pointAt(shape, previous);
  const pointBefore = pointAt(shape, previousPrevious);
  const pointAfter = pointAt(shape, nextOfPrevious);
  const amount = (1 - tension) / 6;
  const c1 = [
    point.x + (pointAfter.x - pointBefore.x) * amount,
    point.y + (pointAfter.y - pointBefore.y) * amount,
  ];
  const targetBefore = pointAt(shape, previous);
  const targetAfter = pointAt(shape, nextOfTarget);
  const c2 = [
    target.x - (targetAfter.x - targetBefore.x) * amount,
    target.y - (targetAfter.y - targetBefore.y) * amount,
  ];
  return [c1, c2];
}

function segmentCommand(shape, targetIndex) {
  const point = pointAt(shape, targetIndex);
  if (point.mode === 'controls' || point.mode === 'controls-relative') {
    return `C${point.c1[0]} ${point.c1[1]} ${point.c2[0]} ${point.c2[1]} ${point.x} ${point.y}`;
  }
  if (point.mode === 'smooth') {
    const [c1, c2] = smoothControls(shape, targetIndex);
    return `C${c1[0]} ${c1[1]} ${c2[0]} ${c2[1]} ${point.x} ${point.y}`;
  }
  if (point.mode === 'arc' && point.arc) {
    return `A${point.arc.rx} ${point.arc.ry} 0 ${point.arc.large ? 1 : 0} ${point.arc.sweep ? 1 : 0} ${point.x} ${point.y}`;
  }
  return `L${point.x} ${point.y}`;
}

export function buildPath(shape) {
  return contours(shape).map(([begin, end]) => {
    let path = `M${shape.points[begin].x} ${shape.points[begin].y}`;
    for (let index = begin + 1; index < end; index++) {
      path += ` ${segmentCommand(shape, index)}`;
    }
    if (shape.closed) path += ` ${segmentCommand(shape, begin)} Z`;
    return path;
  }).join(' ');
}

function cubicMidpoint(p0, c1, c2, p3) {
  const a = [lerp(p0[0], c1[0], .5), lerp(p0[1], c1[1], .5)];
  const b = [lerp(c1[0], c2[0], .5), lerp(c1[1], c2[1], .5)];
  const c = [lerp(c2[0], p3[0], .5), lerp(c2[1], p3[1], .5)];
  const d = [lerp(a[0], b[0], .5), lerp(a[1], b[1], .5)];
  const e = [lerp(b[0], c[0], .5), lerp(b[1], c[1], .5)];
  return [lerp(d[0], e[0], .5), lerp(d[1], e[1], .5)];
}

export function segmentMidpoint(shape, fromIndex) {
  const toIndex = nextIndex(shape, fromIndex);
  if (toIndex === null) return null;
  const from = pointAt(shape, fromIndex);
  const to = pointAt(shape, toIndex);
  if (to.mode === 'controls' || to.mode === 'controls-relative') {
    return cubicMidpoint([from.x, from.y], to.c1, to.c2, [to.x, to.y]);
  }
  if (to.mode === 'smooth') {
    const [c1, c2] = smoothControls(shape, toIndex);
    return cubicMidpoint([from.x, from.y], c1, c2, [to.x, to.y]);
  }
  return [(from.x + to.x) / 2, (from.y + to.y) / 2];
}
