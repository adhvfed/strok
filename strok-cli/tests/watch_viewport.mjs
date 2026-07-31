import assert from 'node:assert/strict';
import test from 'node:test';

import { screenAlignedMarkerTransform, ViewBoxCamera } from '../src/watch_ui/viewport.mjs';

test('zoom keeps its world anchor fixed and clamps to the configured range', () => {
  const camera = new ViewBoxCamera({ minZoom: 0.5, maxZoom: 4 });
  assert.equal(camera.install('shape:a', { x: 0, y: 0, width: 100, height: 80 }), true);

  assert.equal(camera.zoomBy(2, { x: 25, y: 20, fractionX: 0.25, fractionY: 0.25 }), true);
  assert.deepEqual(camera.viewBox, { x: 12.5, y: 10, width: 50, height: 40 });
  assert.equal(camera.zoom, 2);

  camera.zoomBy(100);
  assert.equal(camera.zoom, 4);
  camera.zoomBy(0.0001);
  assert.equal(camera.zoom, 0.5);
});

test('same-content refresh preserves the view while a new target resets to fit', () => {
  const camera = new ViewBoxCamera();
  camera.install('shape:a', { x: 0, y: 0, width: 100, height: 100 });
  camera.zoomBy(2);
  camera.panByWorld(7, -3);

  camera.install('shape:a', { x: 0, y: 0, width: 120, height: 80 });
  assert.equal(camera.zoom, 2);
  assert.deepEqual(camera.viewBox, { x: 27, y: 27, width: 60, height: 40 });

  camera.install('shape:b', { x: 10, y: 20, width: 30, height: 40 });
  assert.equal(camera.zoom, 1);
  assert.deepEqual(camera.viewBox, { x: 10, y: 20, width: 30, height: 40 });
});

test('fit restores the installed bounds and invalid boxes are rejected', () => {
  const camera = new ViewBoxCamera();
  assert.equal(camera.install('bad', { x: 0, y: 0, width: 0, height: 10 }), false);
  assert.equal(camera.ready, false);

  camera.install('document', { x: -5, y: 2, width: 50, height: 25 });
  camera.zoomBy(3);
  assert.equal(camera.fit(), true);
  assert.deepEqual(camera.viewBox, { x: -5, y: 2, width: 50, height: 25 });
});

test('screen-aligned markers cancel non-uniform scale around their center', () => {
  const marker = screenAlignedMarkerTransform(
    { a: 2, b: 0, c: 0, d: 4 },
    10,
    20,
    7,
  );
  assert.deepEqual(marker, [3.5, 0, 0, 1.75, -25, -15]);
  assert.equal(2 * (marker[0] * 10 + marker[2] * 20 + marker[4]), 20);
  assert.equal(4 * (marker[1] * 10 + marker[3] * 20 + marker[5]), 80);
  assert.equal(2 * marker[0], 7);
  assert.equal(4 * marker[3], 7);
});
