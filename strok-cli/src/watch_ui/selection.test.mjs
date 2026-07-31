import assert from 'node:assert/strict';
import test from 'node:test';

import { PointSelection } from './selection.mjs';

test('anchor selection replaces and toggles without losing a stable primary', () => {
  const selection = new PointSelection();
  selection.replaceAnchor('a');
  selection.toggleAnchor('b');

  assert.deepEqual(selection.anchorNames(), ['a', 'b']);
  assert.equal(selection.primaryAnchor, 'b');

  selection.focusAnchor('a');
  assert.equal(selection.primaryAnchor, 'a');

  selection.toggleAnchor('b');
  assert.deepEqual(selection.anchorNames(), ['a']);
  assert.equal(selection.primaryAnchor, 'a');
});

test('controls and anchor sets are mutually exclusive', () => {
  const selection = new PointSelection();
  selection.selectAll(['a', 'b']);
  selection.selectControl('segment', 'c1', 'a');

  assert.equal(selection.anchorCount, 0);
  assert.deepEqual(selection.control, {
    point: 'segment',
    handle: 'c1',
    anchor: 'a',
  });
});

test('reconcile removes points that disappeared after a source refresh', () => {
  const selection = new PointSelection();
  selection.selectAll(['a', 'b', 'c']);
  selection.reconcile(['a']);

  assert.deepEqual(selection.anchorNames(), ['a']);
  assert.equal(selection.primaryAnchor, 'a');
});
