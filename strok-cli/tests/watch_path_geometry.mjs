import assert from 'node:assert/strict';
import test from 'node:test';

import {
  buildPath,
  handlesAtAnchor,
  isRetracted,
  nextIndex,
  oppositeHandle,
  segmentMidpoint,
} from '../src/watch_ui/path-geometry.mjs';

const closedSquare = {
  closed: true,
  points: [
    { name: 'a', x: 0, y: 0, start: true, mode: 'sharp', controlsEditable: false },
    { name: 'b', x: 10, y: 0, start: false, mode: 'sharp', controlsEditable: false },
    { name: 'c', x: 10, y: 10, start: false, mode: 'sharp', controlsEditable: false },
    { name: 'd', x: 0, y: 10, start: false, mode: 'sharp', controlsEditable: false },
  ],
};

test('closed contours wrap topology and serialize their closing segment', () => {
  assert.equal(nextIndex(closedSquare, 3), 0);
  assert.equal(buildPath(closedSquare), 'M0 0 L10 0 L10 10 L0 10 L0 0 Z');
  assert.deepEqual(segmentMidpoint(closedSquare, 3), [0, 5]);
});

test('explicit cubic segments use their true midpoint', () => {
  const shape = {
    closed: false,
    points: [
      { name: 'a', x: 0, y: 0, start: true, mode: 'sharp' },
      { name: 'b', x: 10, y: 0, start: false, mode: 'controls', c1: [0, 10], c2: [10, 10] },
    ],
  };
  assert.equal(buildPath(shape), 'M0 0 C0 10 10 10 10 0');
  assert.deepEqual(segmentMidpoint(shape, 0), [5, 7.5]);
});

test('handle pairing follows the anchor across adjacent segment storage', () => {
  const shape = {
    closed: true,
    points: [
      { name: 'a', x: 0, y: 0, start: true, controlsEditable: true, c1: [-2, 0], c2: [2, 0] },
      { name: 'b', x: 10, y: 0, start: false, controlsEditable: true, c1: [8, 0], c2: [12, 0] },
    ],
  };
  assert.deepEqual(handlesAtAnchor(shape, 0), [
    { index: 0, handle: 'c2' },
    { index: 1, handle: 'c1' },
  ]);
  assert.deepEqual(oppositeHandle(shape, 0, 'c2'), { index: 1, handle: 'c1' });
  assert.equal(isRetracted(shape, 0, 'c2'), false);
  shape.points[0].c2 = [0, 0];
  assert.equal(isRetracted(shape, 0, 'c2'), true);
});
