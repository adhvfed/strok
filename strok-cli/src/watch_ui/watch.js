  import {
    attachedAnchorIndex,
    buildPath,
    contourFor,
    handlesAtAnchor,
    isRetracted,
    nextIndex,
    oppositeHandle,
    pointAt,
    previousIndex,
    segmentMidpoint,
  } from '/path-geometry.js';
  import { PointSelection } from '/selection.js';
  import { screenAlignedMarkerTransform, ViewBoxCamera } from '/viewport.js';

  const $ = (id) => document.getElementById(id);
  const ns = 'http://www.w3.org/2000/svg';
  const backdrops = ['checker', 'white', 'black'];
  let backdrop = 0, version = -1, state = null, editing = false;
  let shapeName = null, activeTarget = null, drag = null;
  let editorSvg = null, editorLayer = null, toastTimer = null;
  const selection = new PointSelection();
  const camera = new ViewBoxCamera();
  let editQueue = Promise.resolve();
  let gestureScale = 1;

  $('bgbtn').onclick = () => {
    $('stage').classList.remove(backdrops[backdrop]);
    backdrop = (backdrop + 1) % backdrops.length;
    $('stage').classList.add(backdrops[backdrop]);
  };
  $('editbtn').onclick = () => editing ? leaveEditor() : enterEditor();
  $('shape-select').onchange = (event) => {
    shapeName = event.target.value;
    activeTarget = state.targets.find((target) => target.shape === shapeName)?.name ?? null;
    selection.clear(); renderEditor();
  };
  $('deletebtn').onclick = deleteSelected;
  $('symmetricbtn').onclick = equalizeSelected;
  $('undobtn').onclick = () => edit({ action: 'undo' });
  $('redobtn').onclick = () => edit({ action: 'redo' });
  $('zoomout').onclick = () => zoomAt(1 / 1.25);
  $('zoomin').onclick = () => zoomAt(1.25);
  $('zoomfit').onclick = fitViewport;

  function showToast(message) {
    $('toast').textContent = message;
    $('toast').classList.add('show');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => $('toast').classList.remove('show'), 4200);
  }

  function stageSvg() { return $('stage').querySelector('svg'); }

  function installViewport(svg, id, fit) {
    if (!camera.install(id, fit)) return;
    ['zoomout', 'zoomfit', 'zoomin'].forEach((button) => { $(button).disabled = false; });
    applyViewport(svg);
  }

  function applyViewport(svg = stageSvg()) {
    if (!svg || !camera.viewBox) return;
    const box = camera.viewBox;
    svg.setAttribute('viewBox', `${box.x} ${box.y} ${box.width} ${box.height}`);
    const percentage = Math.round(camera.zoom * 100);
    $('zoomfit').textContent = `${percentage}%`;
    $('zoomfit').setAttribute('aria-label', `Zoom to fit, current zoom ${percentage} percent`);
    $('zoomout').disabled = camera.zoom <= camera.minZoom;
    $('zoomin').disabled = camera.zoom >= camera.maxZoom;
    if (svg === editorSvg) updateOverlayScale();
  }

  function fitViewport() {
    if (camera.fit()) applyViewport();
  }

  function zoomAt(factor, clientX = null, clientY = null) {
    const svg = stageSvg();
    if (!svg || !camera.viewBox) return;
    let anchor = null;
    if (clientX !== null && clientY !== null) {
      const world = new DOMPoint(clientX, clientY).matrixTransform(svg.getScreenCTM().inverse());
      anchor = {
        x: world.x,
        y: world.y,
        fractionX: (world.x - camera.viewBox.x) / camera.viewBox.width,
        fractionY: (world.y - camera.viewBox.y) / camera.viewBox.height,
      };
    }
    if (camera.zoomBy(factor, anchor)) applyViewport(svg);
  }

  function panViewport(deltaX, deltaY) {
    const svg = stageSvg();
    if (!svg || !camera.viewBox) return;
    const inverse = svg.getScreenCTM().inverse();
    const dx = inverse.a * deltaX + inverse.c * deltaY;
    const dy = inverse.b * deltaX + inverse.d * deltaY;
    if (camera.panByWorld(dx, dy)) applyViewport(svg);
  }

  $('stage').addEventListener('wheel', (event) => {
    if (!stageSvg()) return;
    event.preventDefault();
    const unit = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : 1;
    if (event.ctrlKey || event.metaKey) {
      zoomAt(Math.exp(-event.deltaY * unit * .002), event.clientX, event.clientY);
    } else if (event.shiftKey && event.deltaX === 0) {
      panViewport(event.deltaY * unit, 0);
    } else {
      panViewport(event.deltaX * unit, event.deltaY * unit);
    }
  }, { passive: false });
  $('stage').addEventListener('gesturestart', (event) => {
    if (!stageSvg()) return;
    event.preventDefault(); gestureScale = event.scale || 1;
  }, { passive: false });
  $('stage').addEventListener('gesturechange', (event) => {
    if (!stageSvg()) return;
    event.preventDefault();
    const scale = event.scale || 1;
    zoomAt(scale / gestureScale, event.clientX, event.clientY);
    gestureScale = scale;
  }, { passive: false });
  $('stage').addEventListener('gestureend', (event) => { event.preventDefault(); gestureScale = 1; }, { passive: false });

  async function refresh() {
    try {
      const response = await fetch('/state.json');
      state = await response.json();
      version = state.version;
      document.title = state.file + ' — strøk';
      $('name').textContent = state.file;
      $('rev').textContent = 'rev ' + state.version;
      $('undobtn').disabled = !state.canUndo;
      $('redobtn').disabled = !state.canRedo;
      $('errorbar').hidden = !state.error;
      $('errorbar').textContent = state.error || '';
      $('stage').classList.toggle('stale', !!state.error);
      $('dot').className = 'dot ' + (state.error ? 'err' : 'ok');
      $('editbtn').disabled = !state.editor.length;
      $('editbtn').title = state.editor.length ? 'Edit named shape geometry' : 'This document has no local editable shapes';
      updateShapeSelect();
      if (editing && state.editor.length) renderEditor();
      else if (editing) leaveEditor();
      else renderDocument();
      $('save-status').textContent = '';
    } catch (_) {
      $('dot').className = 'dot';
    }
  }

  function updateShapeSelect() {
    const names = state.editor.map((shape) => shape.name);
    if (!shapeName || !names.includes(shapeName)) shapeName = names[0] || null;
    if (!state.targets.some((target) => target.name === activeTarget && target.shape === shapeName)) {
      activeTarget = state.targets.find((target) => target.shape === shapeName)?.name ?? null;
    }
    $('shape-select').replaceChildren(...names.map((name) => {
      const option = document.createElement('option');
      option.value = name; option.textContent = name; option.selected = name === shapeName;
      return option;
    }));
  }

  function renderDocument() {
    if (!state || !state.svg) return;
    $('stage').innerHTML = state.svg;
    const svg = $('stage').querySelector('svg');
    if (!svg) return;
    const width = svg.getAttribute('width'), height = svg.getAttribute('height');
    if (!svg.getAttribute('viewBox') && width && height) svg.setAttribute('viewBox', `0 0 ${parseFloat(width)} ${parseFloat(height)}`);
    if (width && height) $('size').textContent = parseFloat(width) + '×' + parseFloat(height);
    svg.removeAttribute('width'); svg.removeAttribute('height');
    svg.setAttribute('aria-label', 'Document preview');
    installShapeTargets(svg);
    const box = svg.viewBox.baseVal;
    if (box.width > 0 && box.height > 0) installViewport(svg, 'document', box);
  }

  const matrixValue = (transform) => `matrix(${transform.join(' ')})`;

  function installShapeTargets(svg) {
    const layer = svgElement('g', { 'data-role': 'shape-targets', 'aria-label': 'Editable shapes' });
    state.targets.forEach((target) => {
      const shape = state.editor.find((candidate) => candidate.name === target.shape);
      if (!shape) return;
      const hit = svgElement('path', {
        class: 'shape-target', d: buildPath(shape), transform: matrixValue(target.transform),
        tabindex: '0', role: 'button', 'aria-label': `Edit ${target.shape}, placed as ${target.name}`,
      });
      hit.append(svgElement('title'));
      hit.firstChild.textContent = targetLabel(target);
      const show = (event) => showShapeTip(event, target);
      hit.addEventListener('pointerenter', show);
      hit.addEventListener('pointermove', show);
      hit.addEventListener('pointerleave', hideShapeTip);
      hit.addEventListener('click', (event) => {
        event.preventDefault(); event.stopPropagation();
        editTarget(target);
      });
      hit.addEventListener('keydown', (event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault(); editTarget(target);
        }
      });
      layer.append(hit);
    });
    svg.append(layer);
  }

  function showShapeTip(event, target) {
    const tip = $('shape-tip');
    tip.textContent = targetLabel(target);
    tip.hidden = false;
    tip.style.left = `${Math.min(event.clientX + 12, innerWidth - tip.offsetWidth - 8)}px`;
    tip.style.top = `${Math.min(event.clientY + 12, innerHeight - tip.offsetHeight - 8)}px`;
  }

  const targetLabel = (target) => target.name === target.shape ? target.shape : `${target.shape} · ${target.name}`;
  function hideShapeTip() { $('shape-tip').hidden = true; }

  function editTarget(target) {
    hideShapeTip();
    shapeName = target.shape; activeTarget = target.name; selection.clear();
    updateShapeSelect(); enterEditor();
  }

  function enterEditor() {
    if (!state?.editor.length) return;
    activeTarget ??= state.targets.find((target) => target.shape === shapeName)?.name ?? null;
    editing = true; selection.clear();
    $('inspector').hidden = false;
    $('editbtn').textContent = 'Done';
    $('editbtn').classList.remove('primary');
    renderEditor();
  }

  function leaveEditor() {
    editing = false; drag = null; editorSvg = null; editorLayer = null;
    $('inspector').hidden = true;
    $('editbtn').textContent = 'Edit shape';
    $('editbtn').classList.add('primary');
    $('point-name').textContent = 'Select a point';
    $('deletebtn').disabled = true;
    $('symmetricbtn').disabled = true;
    renderDocument();
  }

  function currentShape() { return state?.editor.find((shape) => shape.name === shapeName); }
  function currentTarget() {
    return state?.targets.find((target) => target.name === activeTarget && target.shape === shapeName);
  }
  function selectedAnchorIndex(shape) {
    const name = selection.control?.anchor ?? selection.primaryAnchor;
    if (!name) return null;
    const index = shape.points.findIndex((point) => point.name === name);
    return index >= 0 ? index : null;
  }
  function isSelectedControl(point, handle) {
    return selection.control?.point === point.name && selection.control.handle === handle;
  }
  function svgElement(tag, attrs = {}) {
    const element = document.createElementNS(ns, tag);
    Object.entries(attrs).forEach(([name, value]) => element.setAttribute(name, value));
    return element;
  }

  function editorMetrics(shape) {
    const coordinates = shape.points.flatMap((point) =>
      [[point.x, point.y], point.c1, point.c2].filter(Boolean),
    );
    const xs = coordinates.map(([x]) => x);
    const ys = coordinates.map(([, y]) => y);
    const minX = Math.min(...xs);
    const maxX = Math.max(...xs);
    const minY = Math.min(...ys);
    const maxY = Math.max(...ys);
    const span = Math.max(maxX - minX, maxY - minY, 1);
    const padding = Math.max(span * 0.14, 8);

    return {
      radius: Math.max(span / 105, 0.8),
      fitBox: {
        x: minX - padding,
        y: minY - padding,
        width: Math.max(maxX - minX, 1) + 2 * padding,
        height: Math.max(maxY - minY, 1) + 2 * padding,
      },
    };
  }

  function appendSnapGuides(svg) {
    const layer = svgElement('g', {
      'data-role': 'guides',
      'aria-hidden': 'true',
    });
    layer.append(
      svgElement('line', {
        class: 'snap-guide',
        'data-guide': 'x',
        visibility: 'hidden',
      }),
      svgElement('line', {
        class: 'snap-guide',
        'data-guide': 'y',
        visibility: 'hidden',
      }),
    );
    svg.append(layer);
  }

  function appendControlLines(svg, shape) {
    shape.points.forEach((point, index) => {
      if (!point.c1 || !point.c2) return;

      const previous = pointAt(shape, previousIndex(shape, index));
      if (!isRetracted(shape, index, 'c1')) {
        svg.append(svgElement('line', {
          class: 'control-line',
          'data-line': `${index}-c1`,
          x1: previous.x,
          y1: previous.y,
          x2: point.c1[0],
          y2: point.c1[1],
        }));
      }
      if (!isRetracted(shape, index, 'c2')) {
        svg.append(svgElement('line', {
          class: 'control-line',
          'data-line': `${index}-c2`,
          x1: point.x,
          y1: point.y,
          x2: point.c2[0],
          y2: point.c2[1],
        }));
      }
    });
  }

  function appendTangentGuide(svg, shape) {
    if (selection.anchorCount > 1) return;
    const anchorIndex = selectedAnchorIndex(shape);
    if (anchorIndex === null) return;

    const handles = handlesAtAnchor(shape, anchorIndex)
      .filter(({ index, handle }) => !isRetracted(shape, index, handle));
    if (handles.length !== 2) return;

    const [firstHandle, secondHandle] = handles;
    const first = pointAt(shape, firstHandle.index)[firstHandle.handle];
    const second = pointAt(shape, secondHandle.index)[secondHandle.handle];
    svg.append(svgElement('line', {
      class: 'tangent-guide',
      'data-role': 'tangent-guide',
      x1: first[0],
      y1: first[1],
      x2: second[0],
      y2: second[1],
    }));
  }

  function activateWithKeyboard(event, action) {
    if (event.key !== 'Enter' && event.key !== ' ') return;
    event.preventDefault();
    action();
  }

  function appendInsertionControls(svg, shape, radius) {
    shape.points.forEach((point, index) => {
      const midpoint = segmentMidpoint(shape, index);
      if (!midpoint) return;

      const [x, y] = midpoint;
      const group = svgElement('g', {
        tabindex: '0',
        role: 'button',
        'aria-label': `Add point after ${point.name}`,
        'data-insert': index,
      });
      group.append(
        svgElement('circle', {
          class: 'insert',
          cx: x,
          cy: y,
          r: radius * 0.82,
        }),
        svgElement('line', {
          class: 'insert-mark',
          x1: x - radius * 0.38,
          y1: y,
          x2: x + radius * 0.38,
          y2: y,
        }),
        svgElement('line', {
          class: 'insert-mark',
          x1: x,
          y1: y - radius * 0.38,
          x2: x,
          y2: y + radius * 0.38,
        }),
      );
      group.onclick = () => addAfter(index);
      group.onkeydown = (event) => activateWithKeyboard(event, () => addAfter(index));
      svg.append(group);
    });
  }

  function appendBezierControls(svg, shape, point, index, radius) {
    if (!point.c1 || !point.c2) return;

    ['c1', 'c2'].forEach((handle) => {
      if (isRetracted(shape, index, handle)) return;

      const selected = isSelectedControl(point, handle) ? ' selected' : '';
      const readonly = point.controlsEditable ? '' : ' readonly';
      const [x, y] = point[handle];
      const anchor = pointAt(shape, attachedAnchorIndex(shape, index, handle));
      const control = svgElement('rect', {
        class: `control${readonly}${selected}`,
        'data-control': `${index}-${handle}`,
        x: x - radius * 0.7,
        y: y - radius * 0.7,
        width: radius * 1.4,
        height: radius * 1.4,
        rx: radius * 0.18,
        tabindex: point.controlsEditable ? '0' : '-1',
        role: 'button',
        'aria-label': `${handle === 'c1' ? 'Outgoing' : 'Incoming'} control for ${anchor.name}`,
      });
      if (point.controlsEditable) {
        control.onpointerdown = (event) => startDrag(event, 'control', index, handle);
        control.onkeydown = (event) =>
          activateWithKeyboard(event, () => selectControl(index, handle));
      }
      svg.append(control);
    });
  }

  function appendAnchorsAndControls(svg, shape, radius) {
    shape.points.forEach((point, index) => {
      appendBezierControls(svg, shape, point, index, radius);

      const selected = selection.hasAnchor(point.name) ? ' selected' : '';
      const anchor = svgElement('circle', {
        class: `anchor${selected}`,
        'data-anchor': index,
        cx: point.x,
        cy: point.y,
        r: radius,
        tabindex: '0',
        role: 'button',
        'aria-pressed': selection.hasAnchor(point.name) ? 'true' : 'false',
        'aria-label': `Point ${point.name}`,
      });
      anchor.onpointerdown = (event) => startDrag(event, 'anchor', index, null);
      anchor.onkeydown = (event) =>
        activateWithKeyboard(event, () => selectAnchor(index, event.shiftKey));
      svg.append(anchor);
    });
  }

  function createEditorOverlay(shape, { fitBox, radius }) {
    const svg = svgElement('svg', {
      role: 'img',
      'aria-label': `Editing ${shape.name}`,
      viewBox: `${fitBox.x} ${fitBox.y} ${fitBox.width} ${fitBox.height}`,
    });
    svg.dataset.baseRadius = radius;
    svg.dataset.guideBox = JSON.stringify(fitBox);

    const path = svgElement('path', {
      class: 'edit-path',
      d: buildPath(shape),
      'data-role': 'path',
    });
    svg.append(path);
    appendSnapGuides(svg);
    appendControlLines(svg, shape);
    appendTangentGuide(svg, shape);
    appendInsertionControls(svg, shape, radius);
    appendAnchorsAndControls(svg, shape, radius);
    return svg;
  }

  function documentContextSvg(shape) {
    if (!state.svg) return null;

    const holder = document.createElement('div');
    holder.innerHTML = state.svg;
    const svg = holder.querySelector('svg');
    if (!svg) return null;

    const width = svg.getAttribute('width');
    const height = svg.getAttribute('height');
    if (!svg.getAttribute('viewBox') && width && height) {
      svg.setAttribute('viewBox', `0 0 ${parseFloat(width)} ${parseFloat(height)}`);
    }
    svg.removeAttribute('width');
    svg.removeAttribute('height');
    svg.setAttribute('aria-label', `Editing ${shape.name} in document context`);
    return svg;
  }

  function mountEditorSvg(shape, target, controlsSvg, { fitBox, radius }) {
    const contextSvg = target ? documentContextSvg(shape) : null;
    if (!contextSvg) {
      return {
        svg: controlsSvg,
        layer: controlsSvg,
        viewportId: `edit:${shape.name}`,
        fitBox,
      };
    }

    const overlay = svgElement('g', {
      transform: matrixValue(target.transform),
      'data-role': 'editor-overlay',
      'aria-label': `Geometry controls for ${shape.name}`,
    });
    overlay.dataset.baseRadius = radius;
    overlay.dataset.guideBox = JSON.stringify(fitBox);
    while (controlsSvg.firstChild) overlay.append(controlsSvg.firstChild);
    contextSvg.append(overlay);

    return {
      svg: contextSvg,
      layer: overlay,
      viewportId: `edit:${target.name}`,
      fitBox: contextSvg.viewBox.baseVal,
    };
  }

  function renderEditor() {
    const shape = currentShape();
    if (!shape) return;
    selection.reconcile(shape.points.map((point) => point.name));

    const metrics = editorMetrics(shape);
    const controlsSvg = createEditorOverlay(shape, metrics);
    const mounted = mountEditorSvg(shape, currentTarget(), controlsSvg, metrics);
    editorSvg = mounted.svg;
    editorLayer = mounted.layer;

    $('stage').replaceChildren(mounted.svg);
    installViewport(mounted.svg, mounted.viewportId, mounted.fitBox);
    updateOverlayScale();
    $('size').textContent = `${shape.points.length} ${shape.points.length === 1 ? 'point' : 'points'}`;
    updatePointPanel();
  }

  function selectAnchor(index, toggle = false) {
    const shape = currentShape(), point = pointAt(shape, index);
    if (toggle) selection.toggleAnchor(point.name);
    else selection.replaceAnchor(point.name);
    renderEditor();
  }

  function selectControl(index, handle) {
    const shape = currentShape(), point = pointAt(shape, index), anchor = pointAt(shape, attachedAnchorIndex(shape, index, handle));
    selection.selectControl(point.name, handle, anchor.name);
    renderEditor();
  }

  function updatePointPanel() {
    const shape = currentShape(), anchorIndex = shape ? selectedAnchorIndex(shape) : null;
    if (anchorIndex === null) {
      $('point-name').textContent = 'Select a point';
      $('point-coords').textContent = 'Drag an anchor to move it';
      $('deletebtn').disabled = true;
      $('symmetricbtn').disabled = true;
      return;
    }
    const anchor = pointAt(shape, anchorIndex);
    const control = selection.control;
    if (control) {
      const segment = shape.points.find((point) => point.name === control.point);
      const value = segment?.[control.handle];
      $('point-name').textContent = `${anchor.name} · ${control.handle === 'c1' ? 'outgoing' : 'incoming'} control`;
      $('point-coords').textContent = value ? `${formatNumber(value[0])}, ${formatNumber(value[1])}` : 'Control no longer exists';
      $('deletebtn').textContent = 'Retract control';
      $('deletebtn').disabled = !value;
    } else if (selection.anchorCount > 1) {
      $('point-name').textContent = `${selection.anchorCount} points selected`;
      $('point-coords').textContent = 'Drag or use arrow keys to move them together';
      $('deletebtn').textContent = 'Delete point';
      $('deletebtn').disabled = true;
      $('symmetricbtn').disabled = true;
      return;
    } else {
      $('point-name').textContent = anchor.name;
      $('point-coords').textContent = `${formatNumber(anchor.x)}, ${formatNumber(anchor.y)} · ${anchor.mode}`;
      const [begin, end] = contourFor(shape, anchorIndex);
      $('deletebtn').textContent = 'Delete point';
      $('deletebtn').disabled = end - begin <= (shape.closed ? 3 : 2);
    }
    $('symmetricbtn').disabled = !anchor.canSymmetrize;
  }
  const formatNumber = (value) => Number(value.toFixed(3)).toString();

  function screenPoint(event) {
    const point = new DOMPoint(event.clientX, event.clientY);
    return point.matrixTransform(editorLayer.getScreenCTM().inverse());
  }

  function startDrag(event, kind, index, handle) {
    event.preventDefault(); event.stopPropagation();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    const shape = currentShape(), point = pointAt(shape, index);
    if (kind === 'anchor') {
      if (event.shiftKey && selection.hasAnchor(point.name)) {
        selection.toggleAnchor(point.name);
        updateSelectionClasses(shape);
        updatePointPanel();
        return;
      }
      if (event.shiftKey) selection.toggleAnchor(point.name);
      else if (!selection.hasAnchor(point.name)) selection.replaceAnchor(point.name);
      else selection.focusAnchor(point.name);
      const anchorNames = selection.anchorNames();
      drag = {
        kind,
        index,
        anchorNames,
        anchorIndices: anchorNames.map((name) => shape.points.findIndex((candidate) => candidate.name === name)),
        startPoints: shape.points.map((candidate) => ({
          x: candidate.x,
          y: candidate.y,
          c1: candidate.c1 ? [...candidate.c1] : candidate.c1,
          c2: candidate.c2 ? [...candidate.c2] : candidate.c2,
        })),
        delta: { x: 0, y: 0 },
        moved: false,
      };
    } else {
      const anchorIndex = attachedAnchorIndex(shape, index, handle), anchor = pointAt(shape, anchorIndex);
      selection.selectControl(point.name, handle, anchor.name);
      const opposite = event.altKey ? null : oppositeHandle(shape, index, handle);
      const oppositePoint = opposite ? pointAt(shape, opposite.index)[opposite.handle] : null;
      drag = {
        kind, index, handle, anchorIndex, opposite, moved: false,
        oppositeLength: oppositePoint ? Math.hypot(oppositePoint[0] - anchor.x, oppositePoint[1] - anchor.y) : 0,
      };
    }
    updateSelectionClasses(shape);
    updateTangentGuide();
    updatePointPanel();
  }

  function updateSelectionClasses(shape) {
    editorLayer.querySelectorAll('.anchor').forEach((anchor) => {
      const point = pointAt(shape, Number(anchor.dataset.anchor));
      const selected = selection.hasAnchor(point.name);
      anchor.classList.toggle('selected', selected);
      anchor.setAttribute('aria-pressed', selected ? 'true' : 'false');
    });
    editorLayer.querySelectorAll('.control').forEach((control) => {
      const [controlIndex, controlHandle] = control.dataset.control.split('-');
      control.classList.toggle(
        'selected',
        selection.control?.point === pointAt(shape, Number(controlIndex)).name
          && selection.control.handle === controlHandle,
      );
    });
  }

  function constrain45(anchor, target) {
    const dx = target.x - anchor.x, dy = target.y - anchor.y, length = Math.hypot(dx, dy);
    if (length < 1e-9) return target;
    const angle = Math.round(Math.atan2(dy, dx) / (Math.PI / 4)) * (Math.PI / 4);
    return { x: anchor.x + Math.cos(angle) * length, y: anchor.y + Math.sin(angle) * length };
  }

  function snapAnchor(shape, index, target, excluded = new Set()) {
    const inverse = editorLayer.getScreenCTM().inverse();
    const threshold = Math.max(Math.hypot(inverse.a, inverse.b), Math.hypot(inverse.c, inverse.d)) * 7;
    let x = target.x, y = target.y, guideX = null, guideY = null, bestX = threshold, bestY = threshold;
    shape.points.forEach((candidate, candidateIndex) => {
      if (candidateIndex === index || excluded.has(candidateIndex)) return;
      const dx = Math.abs(candidate.x - target.x), dy = Math.abs(candidate.y - target.y);
      if (dx < bestX) { bestX = dx; x = candidate.x; guideX = candidate.x; }
      if (dy < bestY) { bestY = dy; y = candidate.y; guideY = candidate.y; }
    });
    showSnapGuides(guideX, guideY);
    return { x, y };
  }

  function previewAnchorDrag(shape, pending, delta) {
    shape.points.forEach((point, index) => {
      const start = pending.startPoints[index];
      point.x = start.x;
      point.y = start.y;
      point.c1 = start.c1 ? [...start.c1] : start.c1;
      point.c2 = start.c2 ? [...start.c2] : start.c2;
    });
    pending.anchorIndices.forEach((index) => {
      const point = pointAt(shape, index), start = pending.startPoints[index];
      point.x = start.x + delta.x;
      point.y = start.y + delta.y;
      if (point.controlsEditable && start.c2) {
        point.c2 = [start.c2[0] + delta.x, start.c2[1] + delta.y];
      }
      const next = nextIndex(shape, index);
      if (next !== null) {
        const following = pointAt(shape, next), followingStart = pending.startPoints[next];
        if (following.controlsEditable && followingStart.c1) {
          following.c1 = [followingStart.c1[0] + delta.x, followingStart.c1[1] + delta.y];
        }
      }
    });
  }

  function showSnapGuides(x, y) {
    if (!editorLayer) return;
    const box = JSON.parse(editorLayer.dataset.guideBox), xGuide = editorLayer.querySelector('[data-guide="x"]'), yGuide = editorLayer.querySelector('[data-guide="y"]');
    xGuide.setAttribute('visibility', x === null ? 'hidden' : 'visible');
    if (x !== null) { xGuide.setAttribute('x1', x); xGuide.setAttribute('x2', x); xGuide.setAttribute('y1', box.y); xGuide.setAttribute('y2', box.y + box.height); }
    yGuide.setAttribute('visibility', y === null ? 'hidden' : 'visible');
    if (y !== null) { yGuide.setAttribute('x1', box.x); yGuide.setAttribute('x2', box.x + box.width); yGuide.setAttribute('y1', y); yGuide.setAttribute('y2', y); }
  }

  window.addEventListener('pointermove', (event) => {
    if (!drag || !editing || !editorLayer) return;
    const shape = currentShape(), point = pointAt(shape, drag.index);
    let target = screenPoint(event);
    drag.moved = true;
    if (drag.kind === 'anchor') {
      target = snapAnchor(shape, drag.index, target, new Set(drag.anchorIndices));
      const start = drag.startPoints[drag.index];
      drag.delta = { x: target.x - start.x, y: target.y - start.y };
      previewAnchorDrag(shape, drag, drag.delta);
    } else {
      const anchor = pointAt(shape, drag.anchorIndex);
      if (event.shiftKey) target = constrain45(anchor, target);
      point[drag.handle] = [target.x, target.y];
      if (drag.opposite) {
        const dx = target.x - anchor.x, dy = target.y - anchor.y, length = Math.hypot(dx, dy);
        if (length > 1e-9) {
          const oppositePoint = pointAt(shape, drag.opposite.index);
          oppositePoint[drag.opposite.handle] = [anchor.x - dx / length * drag.oppositeLength, anchor.y - dy / length * drag.oppositeLength];
        }
      }
    }
    updateGeometry(); updatePointPanel();
  });

  window.addEventListener('pointerup', async () => {
    if (!drag) return;
    const shape = currentShape(), point = pointAt(shape, drag.index), pending = drag;
    drag = null;
    showSnapGuides(null, null);
    if (!pending.moved) return;
    if (pending.kind === 'anchor') {
      await edit({
        action: 'move-anchors',
        shape: shape.name,
        points: pending.anchorNames.join(','),
        dx: pending.delta.x,
        dy: pending.delta.y,
      });
    }
    else {
      const fields = { action: 'control', shape: shape.name, point: point.name, handle: pending.handle, x: point[pending.handle][0], y: point[pending.handle][1] };
      if (pending.opposite) {
        const oppositePoint = pointAt(shape, pending.opposite.index);
        Object.assign(fields, { oppositePoint: oppositePoint.name, oppositeHandle: pending.opposite.handle, oppositeX: oppositePoint[pending.opposite.handle][0], oppositeY: oppositePoint[pending.opposite.handle][1] });
      }
      await edit(fields);
    }
  });

  window.addEventListener('pointercancel', () => {
    if (!drag) return;
    drag = null;
    showSnapGuides(null, null);
    refresh();
  });

  function updateOverlayScale() {
    if (!editorLayer?.dataset.baseRadius || !editorLayer.getScreenCTM()) return;
    const shape = currentShape();
    editorLayer.querySelectorAll('[data-anchor]').forEach((anchor) => {
      const point = pointAt(shape, Number(anchor.dataset.anchor));
      anchor.setAttribute('r', 1);
      anchor.setAttribute('transform', screenMarkerTransform(point.x, point.y, 7));
    });
    editorLayer.querySelectorAll('[data-control]').forEach((control) => {
      const [index, handle] = control.dataset.control.split('-');
      const center = pointAt(shape, Number(index))[handle], side = 1.4;
      control.setAttribute('x', center[0] - side / 2); control.setAttribute('y', center[1] - side / 2);
      control.setAttribute('width', side); control.setAttribute('height', side); control.setAttribute('rx', .18);
      control.setAttribute('transform', screenMarkerTransform(center[0], center[1], 7));
    });
    editorLayer.querySelectorAll('[data-insert]').forEach((insert) => {
      const index = Number(insert.dataset.insert), midpoint = segmentMidpoint(shape, index);
      if (!midpoint) return;
      const circle = insert.querySelector('circle'), lines = insert.querySelectorAll('line');
      const circleRadius = .82, halfMark = circleRadius * .46;
      circle.setAttribute('r', circleRadius); circle.setAttribute('cx', midpoint[0]); circle.setAttribute('cy', midpoint[1]);
      lines[0].setAttribute('x1', midpoint[0] - halfMark); lines[0].setAttribute('x2', midpoint[0] + halfMark); lines[0].setAttribute('y1', midpoint[1]); lines[0].setAttribute('y2', midpoint[1]);
      lines[1].setAttribute('x1', midpoint[0]); lines[1].setAttribute('x2', midpoint[0]); lines[1].setAttribute('y1', midpoint[1] - halfMark); lines[1].setAttribute('y2', midpoint[1] + halfMark);
      insert.setAttribute('transform', screenMarkerTransform(midpoint[0], midpoint[1], 7));
    });
  }

  function screenMarkerTransform(x, y, pixels) {
    const transform = screenAlignedMarkerTransform(editorLayer.getScreenCTM(), x, y, pixels);
    return transform ? matrixValue(transform) : '';
  }

  function updateGeometry() {
    const shape = currentShape();
    editorLayer.querySelector('[data-role="path"]').setAttribute('d', buildPath(shape));
    shape.points.forEach((point, index) => {
      const anchor = editorLayer.querySelector(`[data-anchor="${index}"]`);
      anchor.setAttribute('cx', point.x); anchor.setAttribute('cy', point.y);
      const mid = segmentMidpoint(shape, index), insert = editorLayer.querySelector(`[data-insert="${index}"]`);
      if (mid && insert) {
        const circle = insert.querySelector('circle'), lines = insert.querySelectorAll('line'), r = Number(circle.getAttribute('r'));
        circle.setAttribute('cx', mid[0]); circle.setAttribute('cy', mid[1]);
        lines[0].setAttribute('x1', mid[0] - r * .46); lines[0].setAttribute('x2', mid[0] + r * .46); lines[0].setAttribute('y1', mid[1]); lines[0].setAttribute('y2', mid[1]);
        lines[1].setAttribute('x1', mid[0]); lines[1].setAttribute('x2', mid[0]); lines[1].setAttribute('y1', mid[1] - r * .46); lines[1].setAttribute('y2', mid[1] + r * .46);
      }
      if (!point.c1 || !point.c2) return;
      ['c1', 'c2'].forEach((handle) => {
        const control = editorLayer.querySelector(`[data-control="${index}-${handle}"]`);
        if (!control) return;
        const width = Number(control.getAttribute('width'));
        control.setAttribute('x', point[handle][0] - width / 2); control.setAttribute('y', point[handle][1] - width / 2);
      });
      const previous = pointAt(shape, previousIndex(shape, index));
      const line1 = editorLayer.querySelector(`[data-line="${index}-c1"]`), line2 = editorLayer.querySelector(`[data-line="${index}-c2"]`);
      if (line1) { line1.setAttribute('x1', previous.x); line1.setAttribute('y1', previous.y); line1.setAttribute('x2', point.c1[0]); line1.setAttribute('y2', point.c1[1]); }
      if (line2) { line2.setAttribute('x1', point.x); line2.setAttribute('y1', point.y); line2.setAttribute('x2', point.c2[0]); line2.setAttribute('y2', point.c2[1]); }
    });
    updateTangentGuide();
    updateOverlayScale();
  }

  function updateTangentGuide() {
    if (!editorLayer) return;
    if (selection.anchorCount > 1) {
      editorLayer.querySelector('[data-role="tangent-guide"]')?.remove();
      return;
    }
    const shape = currentShape(), anchorIndex = selectedAnchorIndex(shape);
    let guide = editorLayer.querySelector('[data-role="tangent-guide"]');
    const pair = anchorIndex === null ? [] : handlesAtAnchor(shape, anchorIndex).filter((item) => !isRetracted(shape, item.index, item.handle));
    if (pair.length !== 2) { guide?.remove(); return; }
    if (!guide) { guide = svgElement('line', { class: 'tangent-guide', 'data-role': 'tangent-guide' }); editorLayer.append(guide); }
    const first = pointAt(shape, pair[0].index)[pair[0].handle], second = pointAt(shape, pair[1].index)[pair[1].handle];
    guide.setAttribute('x1', first[0]); guide.setAttribute('y1', first[1]); guide.setAttribute('x2', second[0]); guide.setAttribute('y2', second[1]);
  }

  async function addAfter(index) {
    const shape = currentShape();
    await edit({ action: 'add', shape: shape.name, after: pointAt(shape, index).name });
  }
  async function deleteSelected() {
    const shape = currentShape();
    if ($('deletebtn').disabled) return;
    const control = selection.control;
    const point = selection.primaryAnchor;
    selection.clear();
    if (control) {
      await edit({ action: 'retract-control', shape: shape.name, point: control.point, handle: control.handle });
    } else {
      await edit({ action: 'delete', shape: shape.name, point });
    }
  }

  async function equalizeSelected() {
    const shape = currentShape(), anchorIndex = shape ? selectedAnchorIndex(shape) : null;
    if (anchorIndex === null || $('symmetricbtn').disabled) return;
    const anchor = pointAt(shape, anchorIndex);
    selection.replaceAnchor(anchor.name);
    await edit({ action: 'symmetric', shape: shape.name, point: anchor.name });
  }

  function edit(fields) {
    editQueue = editQueue.then(() => performEdit(fields));
    return editQueue;
  }

  async function performEdit(fields) {
    $('save-status').textContent = 'Saving…';
    try {
      const response = await fetch('/edit', { method: 'POST', headers: { 'Content-Type': 'application/x-www-form-urlencoded;charset=UTF-8' }, body: new URLSearchParams(fields) });
      if (!response.ok) throw new Error(await response.text());
      $('save-status').textContent = 'Saved';
      await refresh();
    } catch (error) {
      $('save-status').textContent = 'Not saved';
      showToast(error.message || 'The edit could not be saved.');
      await refresh();
    }
  }

  window.addEventListener('keydown', (event) => {
    if (/^(INPUT|SELECT|TEXTAREA)$/.test(event.target.tagName)) return;
    const command = event.metaKey || event.ctrlKey;
    if (!command && (event.key === '+' || event.key === '=')) { event.preventDefault(); zoomAt(1.25); return; }
    if (!command && (event.key === '-' || event.key === '_')) { event.preventDefault(); zoomAt(1 / 1.25); return; }
    if (!command && event.key === '0') { event.preventDefault(); fitViewport(); return; }
    if (command && event.key.toLowerCase() === 'z') {
      event.preventDefault();
      edit({ action: event.shiftKey ? 'redo' : 'undo' });
      return;
    }
    if (!editing) return;
    if (command && event.key.toLowerCase() === 'a') {
      event.preventDefault();
      const shape = currentShape();
      selection.selectAll(shape.points.map((point) => point.name));
      renderEditor();
      return;
    }
    if (!command && selection.anchorCount > 0 && event.key.startsWith('Arrow')) {
      const directions = {
        ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1],
      };
      const direction = directions[event.key];
      if (direction) {
        event.preventDefault();
        const distance = event.altKey ? .1 : (event.shiftKey ? 10 : 1);
        edit({
          action: 'move-anchors', shape: currentShape().name, points: selection.anchorNames().join(','),
          dx: direction[0] * distance, dy: direction[1] * distance,
        });
        return;
      }
    }
    if ((event.key === 'Delete' || event.key === 'Backspace') && !$('deletebtn').disabled) { event.preventDefault(); deleteSelected(); }
    if (event.shiftKey && !command && event.key.toLowerCase() === 'c' && !$('symmetricbtn').disabled) { event.preventDefault(); equalizeSelected(); }
    if (event.key === 'Escape') {
      if (selection.anchorCount || selection.control) {
        selection.clear();
        renderEditor();
      } else {
        leaveEditor();
      }
    }
  });

  const es = new EventSource('/events');
  es.onmessage = (event) => { if (Number(event.data) !== version) refresh(); };
  es.onerror = () => { $('dot').className = 'dot'; };
  refresh();
