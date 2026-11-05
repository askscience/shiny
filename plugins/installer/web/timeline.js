import {
  GIB, MIB, MIN_ROOT_BYTES,
  MODE_ERASE, MODE_EXISTING, MODE_MANUAL,
  buildPlanModel, partitionOs, resizeBoundary, rootCandidates,
} from './timeline-model.js';

const LAYOUT_FIELDS = [
  'manualSwapGiB', 'manualHome', 'manualHomeGiB', 'manualRootGiB', 'swapGiB', 'homeGiB',
];

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function clear(node) {
  if (node) node.textContent = '';
}

function copyLayout(target, source) {
  for (const key of LAYOUT_FIELDS) {
    if (key in source) target[key] = source[key];
  }
}

function pct(value, total) {
  return total > 0 ? (value / total) * 100 : 0;
}

function compact(bytes, formatBytes) {
  if (!bytes) return '0';
  if (formatBytes) return formatBytes(bytes);
  if (bytes >= GIB) return `${Math.round(bytes / GIB)} GiB`;
  return `${Math.round(bytes / MIB)} MiB`;
}

function plannedLabel(kind, t) {
  if (kind === 'efi') return t('disk.timeline.plannedEfi', 'Startup');
  if (kind === 'swap') return t('disk.timeline.plannedSwap', 'Support memory');
  if (kind === 'home') return t('disk.timeline.plannedHome', 'Personal files');
  return t('disk.timeline.plannedRoot', "LiteAura system");
}

function boundaryEdge(model, id) {
  const planned = model.after.filter((segment) => segment.planned);
  const root = planned.find((segment) => segment.kind === 'root');
  if (!root) return null;
  if (id === 'root-end') return root.start + root.size;
  if (id === 'home-root') return root.start;
  const swap = planned.find((segment) => segment.kind === 'swap');
  if (!swap) return root.start;
  const next = planned.find((segment) => segment.start === swap.start + swap.size);
  return next ? next.start : swap.start + swap.size;
}

function boundaryInfo(model, id) {
  const { usable, efi, swap, home, root } = model.sizes;
  if (id === 'swap-root') {
    return {
      value: swap,
      min: 0,
      max: Math.max(0, usable - efi - home - MIN_ROOT_BYTES),
      label: 'swap',
    };
  }
  if (id === 'home-root') {
    return {
      value: home,
      min: GIB,
      max: Math.max(GIB, usable - efi - swap - MIN_ROOT_BYTES),
      label: 'home',
    };
  }
  return {
    value: root,
    min: MIN_ROOT_BYTES,
    max: Math.max(MIN_ROOT_BYTES, usable - efi - swap - home),
    label: 'root',
  };
}

const OS_NAMES = {
  windows: ['disk.os.windows', 'Windows'],
  macos: ['disk.os.macos', 'macOS'],
  linux: ['disk.os.linux', 'Linux'],
  encrypted: ['disk.os.encrypted', 'Encrypted'],
  startup: ['disk.timeline.plannedEfi', 'Startup'],
  swap: ['disk.timeline.plannedSwap', 'Support memory'],
  data: ['disk.os.data', 'Data'],
  empty: ['disk.os.empty', 'Empty'],
};

/** "Windows", "macOS", "Linux"… for a system code from partitionOs/diskOs. */
export function osLabel(os, t) {
  const entry = OS_NAMES[os];
  return entry ? t(entry[0], entry[1]) : '';
}

/** A human name for a partition: the system that owns it, else its filesystem. */
export function partitionLabel(part, t) {
  if (!part) return '';
  const os = partitionOs(part);
  if (os !== 'other') return osLabel(os, t);
  return String(part.fstype || part.name || part.path || '').toUpperCase();
}

export function renderTimelinePanel(options) {
  const {
    disk, state, model: initialModel, t, formatBytes, icon,
    onSelectGap, onSelectPartition, onToggle, onRemovePartition,
    onCommit, onRequestRender, confirm = null,
  } = options;
  const wrap = el('div', 'installer-timeline');
  let model = initialModel || buildPlanModel(disk, state);

  const scroll = el('div', 'installer-timeline-scroll');
  scroll.dir = 'ltr';
  const content = el('div', 'installer-timeline-content');
  scroll.appendChild(content);
  wrap.appendChild(scroll);

  const nowTitle = el('div', 'installer-track-title');
  nowTitle.textContent = t('disk.timeline.now', 'Now');
  content.appendChild(nowTitle);
  const nowTrack = el('div', 'installer-track installer-track--now');
  nowTrack.setAttribute('role', 'group');
  nowTrack.setAttribute('aria-label', t('disk.timeline.now', 'Now'));
  content.appendChild(nowTrack);

  const afterTitle = el('div', 'installer-track-title');
  afterTitle.textContent = t('disk.timeline.after', 'After installation');
  content.appendChild(afterTitle);
  const afterTrack = el('div', 'installer-track installer-track--after');
  afterTrack.setAttribute('role', 'group');
  afterTrack.setAttribute('aria-label', t('disk.timeline.after', 'After installation'));
  const afterWrap = el('div', 'installer-track-wrap');
  afterWrap.appendChild(afterTrack);
  content.appendChild(afterWrap);

  const handleLayer = el('div', 'installer-handle-layer');
  afterWrap.appendChild(handleLayer);

  const handles = new Map();
  const bubbles = new Map();
  function addHandle(id) {
    const handle = el('button', 'installer-handle');
    handle.type = 'button';
    handle.dataset.boundary = id;
    handle.setAttribute('role', 'separator');
    handle.setAttribute('aria-orientation', 'vertical');
    handle.tabIndex = 0;
    handle.appendChild(el('span', 'installer-handle-grip'));
    // The size being dragged, shown above the grip while hovering or dragging.
    const bubble = el('span', 'installer-handle-bubble');
    bubble.setAttribute('aria-hidden', 'true');
    handle.appendChild(bubble);
    bubbles.set(id, bubble);
    handle.addEventListener('pointerdown', (event) => startDrag(event, id));
    handle.addEventListener('keydown', (event) => onHandleKey(event, id));
    handleLayer.appendChild(handle);
    handles.set(id, handle);
  }

  if (state.partitionMode === MODE_ERASE || state.partitionMode === MODE_MANUAL) {
    if (model.sizes.swap > 0) addHandle('swap-root');
    if (model.sizes.home > 0) addHandle('home-root');
    if (state.partitionMode === MODE_MANUAL && model.sizes.root > 0) addHandle('root-end');
  }

  // Formatting an existing partition is confirmed right under the bars.
  if (confirm) wrap.appendChild(confirm);

  // One outcome line: what survives (or what is lost) and how big LiteAura gets.
  const outcome = el('div', 'installer-outcome');
  const outcomeMark = el('span', 'installer-outcome-mark');
  outcomeMark.setAttribute('aria-hidden', 'true');
  const outcomeText = el('span', 'installer-outcome-text');
  const outcomeSize = el('span', 'installer-outcome-size');
  outcome.append(outcomeMark, outcomeText, outcomeSize);
  wrap.appendChild(outcome);
  const summaryErrors = el('div', 'installer-summary-errors');
  wrap.appendChild(summaryErrors);

  // Two switches instead of presets and size tables: sizes are dragged.
  const toggles = el('div', 'installer-toggles');
  toggles.setAttribute('role', 'group');
  const toggleRefs = {};
  if (state.partitionMode !== MODE_EXISTING) {
    for (const [kind, key, fallback] of [
      ['swap', 'disk.timeline.plannedSwap', 'Support memory'],
      ['home', 'disk.toggle.home', 'Separate personal files'],
    ]) {
      const button = el('button', 'installer-toggle');
      button.type = 'button';
      button.setAttribute('role', 'switch');
      const knob = el('span', 'installer-toggle-knob');
      knob.setAttribute('aria-hidden', 'true');
      button.append(knob, el('span', 'installer-toggle-label', t(key, fallback)));
      button.addEventListener('click', () => {
        onToggle?.(kind, button.getAttribute('aria-checked') !== 'true');
      });
      toggleRefs[kind] = button;
      toggles.appendChild(button);
    }
    wrap.appendChild(toggles);
  }

  const live = el('div', 'installer-sr-only');
  live.setAttribute('aria-live', 'polite');
  wrap.appendChild(live);

  function announce(message) {
    live.textContent = message;
  }

  function currentGap() {
    return model.gap || (model.gaps || [])[0] || null;
  }

  /** Will this existing partition be wiped? (Shown red on the "now" bar.) */
  function isDoomed(segment) {
    if (!segment.part) return false;
    if (model.mode === MODE_ERASE) return true;
    return model.mode === MODE_EXISTING && segment.part.path === state.existingRootPath;
  }

  function segmentClass(segment, track) {
    if (segment.kind === 'free') return 'installer-clip installer-clip--free';
    if (segment.planned) return `installer-clip installer-clip--planned installer-clip--${segment.kind}`;
    if (track === 'now' && isDoomed(segment)) return 'installer-clip installer-clip--doomed';
    if (segment.selectedRoot) return 'installer-clip installer-clip--formatted';
    if (segment.reused) return 'installer-clip installer-clip--reused';
    return `installer-clip installer-clip--kept installer-clip--${segment.kind}`;
  }

  function segmentLabel(segment) {
    if (segment.planned) return plannedLabel(segment.kind, t);
    if (segment.kind === 'free') return t('disk.free', 'Free space');
    if (segment.selectedRoot) return t('disk.timeline.plannedRoot', "LiteAura system");
    if (segment.reused) return t('disk.timeline.plannedEfi', 'Startup');
    return partitionLabel(segment.part, t) || segment.label || '';
  }

  function clipBody(segment, locked) {
    const body = el('span', 'installer-clip-copy');
    if (locked && icon) {
      const mark = el('span', 'installer-clip-lock');
      mark.setAttribute('aria-hidden', 'true');
      mark.appendChild(icon('ui/lock', { size: 10 }));
      body.appendChild(mark);
    }
    body.append(
      el('span', 'installer-clip-label', segmentLabel(segment)),
      el('span', 'installer-clip-size', compact(segment.size, formatBytes)),
    );
    return body;
  }

  function appendSegment(track, segment, which) {
    const widthPct = pct(segment.size, disk.size_bytes);
    const isGap = segment.kind === 'free';
    const gapRecord = isGap && (model.gaps || []).find((gap) => (
      gap.start_bytes === segment.start && gap.size_bytes === segment.size
    ));
    const canSelectGap = state.partitionMode !== MODE_ERASE
      && gapRecord?.eligible && segment.source === 'probe';
    const selected = isGap && model.gap && segment.start === model.gap.start_bytes;
    const label = segmentLabel(segment);
    let node;
    if (isGap && canSelectGap) {
      node = el('button', segmentClass(segment, which));
      node.type = 'button';
      node.setAttribute('aria-label', `${t('disk.timeline.gap', 'Free space')} ${compact(segment.size, formatBytes)} — ${t('disk.timeline.installHere', 'Install here')}`);
      node.addEventListener('click', () => onSelectGap?.(segment.gap || {
        start_bytes: segment.start,
        size_bytes: segment.size,
      }));
    } else if (segment.part && state.partitionMode !== MODE_ERASE
      && rootCandidates(disk).some((part) => part.path === segment.part.path)) {
      node = el('button', segmentClass(segment, which));
      node.type = 'button';
      node.setAttribute('aria-label', `${label} ${compact(segment.size, formatBytes)} — ${t('disk.existing.pickOne', 'Install LiteAura in this partition')}`);
      node.addEventListener('click', () => onSelectPartition?.(segment.part));
    } else {
      node = el('div', segmentClass(segment, which));
    }
    node.style.left = `${pct(segment.start, disk.size_bytes)}%`;
    node.style.width = `${widthPct}%`;
    if (widthPct < 4) node.classList.add('is-tiny');
    if (selected) node.classList.add('is-selected');
    const locked = !!segment.part && !segment.selectedRoot && !segment.reused
      && !(which === 'now' && isDoomed(segment));
    if (isGap) {
      // Only point at a gap when there is more than one to choose from.
      if (canSelectGap && !selected && which === 'now' && eligibleGapCount() > 1) {
        node.appendChild(el('span', 'installer-gap-action', t('disk.timeline.installHere', 'Install here')));
      }
      node.appendChild(clipBody(segment, false));
      if (selected) {
        node.setAttribute('aria-current', 'true');
      }
    } else {
      node.appendChild(clipBody(segment, locked));
      if (segment.planned && (segment.kind === 'swap' || segment.kind === 'home')) {
        const remove = el('span', 'installer-clip-remove', '×');
        remove.setAttribute('aria-hidden', 'true');
        const button = el('button', 'installer-clip-remove-button');
        button.type = 'button';
        button.setAttribute('aria-label', t('disk.timeline.removePartition', 'Remove this partition'));
        button.appendChild(remove);
        button.addEventListener('click', (event) => {
          event.stopPropagation();
          onRemovePartition?.(segment.kind);
        });
        node.appendChild(button);
      }
    }
    const detail = segment.part?.path || '';
    node.title = `${label} · ${compact(segment.size, formatBytes)}${detail ? ` · ${detail}` : ''}`;
    track.appendChild(node);
  }

  function eligibleGapCount() {
    return (model.gaps || []).filter((gap) => gap.eligible).length;
  }

  function paint(nextModel) {
    model = nextModel;

    clear(nowTrack);
    for (const segment of model.now) appendSegment(nowTrack, segment, 'now');
    clear(afterTrack);
    for (const segment of model.after) appendSegment(afterTrack, segment, 'after');

    for (const [id, handle] of handles) {
      const info = boundaryInfo(model, id);
      const edge = boundaryEdge(model, id);
      if (edge == null) {
        handle.style.display = 'none';
        continue;
      }
      const edgePct = pct(edge, disk.size_bytes);
      handle.style.display = '';
      handle.style.left = `${edgePct}%`;
      handle.classList.toggle('at-start', edgePct <= 0.05);
      handle.classList.toggle('at-end', edgePct >= 99.95);
      handle.setAttribute('aria-valuemin', String(Math.round(info.min / GIB)));
      handle.setAttribute('aria-valuemax', String(Math.round(info.max / GIB)));
      handle.setAttribute('aria-valuenow', String(Math.round(info.value / GIB)));
      handle.setAttribute('aria-valuetext', `${t(`disk.timeline.planned${info.label === 'swap' ? 'Swap' : info.label === 'home' ? 'Home' : 'Root'}`, info.label)} ${Math.round(info.value / GIB)} GiB`);
      handle.title = t('disk.timeline.keyboardHint', 'Drag or use the arrow keys');
      const bubble = bubbles.get(id);
      if (bubble) {
        bubble.textContent = `${plannedLabel(info.label === 'swap' ? 'swap' : info.label === 'home' ? 'home' : 'root', t)} · ${compact(info.value, formatBytes)}`;
      }
    }

    paintSummary();
    paintToggles();
  }

  function paintSummary() {
    const result = outcomeFor();
    outcome.className = `installer-outcome installer-outcome--${result.tone}`;
    outcome.hidden = !result.text;
    outcomeMark.textContent = result.tone === 'danger' ? '!' : result.tone === 'ok' ? '✓' : '→';
    outcomeText.textContent = result.text;
    outcomeSize.textContent = result.size
      ? `LiteAura ${compact(result.size, formatBytes)}`
      : '';
    clear(summaryErrors);
    for (const error of model.errors) {
      summaryErrors.appendChild(el('p', 'installer-summary-error installer-inline-error', errorLabel(error)));
    }
    if (model.approx) {
      summaryErrors.appendChild(el('p', 'installer-summary-warn', t('disk.timeline.noOffsets', 'Partition positions are approximate; refresh the disks before changing the layout.')));
    }
  }

  /** The single sentence under the bars: what is kept, or what is lost. */
  function outcomeFor() {
    const pendingExisting = state.partitionMode === MODE_EXISTING
      && state.pendingExistingPath && !state.existingConfirmed;
    if (pendingExisting) return { tone: 'none', text: '', size: 0 };
    const rootSize = model.mode === MODE_EXISTING
      ? Number(model.formatted[0]?.size_bytes) || 0
      : model.sizes.root;
    const size = model.errors.length ? 0 : rootSize;

    if (model.mode === MODE_ERASE) {
      if (!(disk.children || []).length) {
        return { tone: 'ok', text: t('disk.outcome.emptyDisk', 'The disk is empty: nothing is lost'), size };
      }
      return { tone: 'danger', text: t('disk.outcome.eraseAll', 'Everything on this disk will be erased'), size };
    }
    if (model.mode === MODE_EXISTING) {
      const target = model.formatted[0];
      if (!target) return { tone: 'none', text: t('disk.outcome.pickPartition', 'Choose the partition to replace'), size: 0 };
      return {
        tone: 'danger',
        text: t('disk.outcome.replace', '{name} will be erased, the rest stays', {
          name: `${partitionLabel(target, t)} ${compact(Number(target.size_bytes) || 0, formatBytes)}`,
        }),
        size,
      };
    }
    if (!model.gap) return { tone: 'none', text: t('disk.outcome.pickGap', 'Choose a free space'), size: 0 };
    const names = [...new Set(model.kept
      .filter((part) => !['startup', 'swap', 'empty'].includes(partitionOs(part)))
      .map((part) => partitionLabel(part, t))
      .filter(Boolean))];
    let text;
    if (!names.length) text = t('disk.outcome.nothingTouched', 'Nothing on the disk is touched');
    else if (names.length === 1) text = t('disk.outcome.keptOne', '{name} stays untouched', { name: names[0] });
    else text = t('disk.outcome.keptMany', '{names} stay untouched', { names: names.join(' · ') });
    return { tone: 'ok', text, size };
  }

  function freeAfterBytes() {
    return Math.max(0, model.remaining || 0);
  }

  function errorLabel(code) {
    const values = {
      noDisk: t('disk.noDisk', 'No disk selected'),
      noFree: t('disk.manual.noFree', 'No usable free space found on this disk'),
      noOffsets: t('disk.timeline.noOffsets', 'Partition positions are approximate; refresh the disks before changing the layout.'),
      gapTooSmall: t('disk.timeline.gapTooSmall', 'The free space is too small for LiteAura.'),
      swapTooBig: t('disk.timeline.swapTooBig', 'The support memory does not fit in this free space.'),
      homeTooBig: t('disk.timeline.homeTooBig', 'The personal-files partition does not fit in this free space.'),
      rootTooBig: t('disk.timeline.rootTooBig', 'The system does not fit in this free space.'),
      rootSmall: t('disk.manual.rootSmall', 'Leave at least 20 GiB for the system.'),
      noRoot: t('disk.existing.noRoot', 'Choose a partition for LiteAura (ext4 or empty, not in use)'),
      noEsp: t('disk.existing.noEsp', 'No startup partition found on this disk'),
    };
    return values[code] || code;
  }

  function paintToggles() {
    for (const [kind, button] of Object.entries(toggleRefs)) {
      const on = model.sizes[kind] > 0;
      button.setAttribute('aria-checked', String(on));
      button.classList.toggle('is-on', on);
    }
  }

  function commit(previewState, previewModel) {
    copyLayout(state, previewState);
    paint(previewModel);
    onCommit?.(state, previewModel);
    announce(t('disk.timeline.announce', 'Layout updated: system {root}, free {free}', {
      root: compact(previewModel.sizes.root, formatBytes),
      free: compact(freeAfterBytes(), formatBytes),
    }));
  }

  function applyBoundary(boundaryId, deltaBytes) {
    const previewState = { ...state };
    const preview = resizeBoundary(disk, previewState, boundaryId, deltaBytes);
    commit(previewState, preview);
  }

  function onHandleKey(event, id) {
    const step = event.shiftKey ? 8 * GIB : GIB;
    const info = boundaryInfo(model, id);
    let delta = 0;
    if (event.key === 'ArrowRight' || event.key === 'ArrowUp') delta = step;
    else if (event.key === 'ArrowLeft' || event.key === 'ArrowDown') delta = -step;
    else if (event.key === 'Home') delta = info.min - info.value;
    else if (event.key === 'End') delta = info.max - info.value;
    else return;
    event.preventDefault();
    applyBoundary(id, delta);
  }

  let drag = null;

  function startDrag(event, id) {
    if (event.button !== 0 && event.pointerType !== 'touch') return;
    event.preventDefault();
    const rect = content.getBoundingClientRect();
    if (!rect.width) return;
    drag = {
      id,
      pointerId: event.pointerId,
      startX: event.clientX,
      startState: { ...state },
      rect,
      handle: event.currentTarget,
      moved: false,
    };
    event.currentTarget.setPointerCapture?.(event.pointerId);
    event.currentTarget.classList.add('is-dragging');
    window.addEventListener('pointermove', onDragMove);
    window.addEventListener('pointerup', endDrag);
    window.addEventListener('pointercancel', cancelDrag);
    window.addEventListener('keydown', onDragKey);
  }

  function dragDelta(clientX) {
    const raw = ((clientX - drag.startX) / drag.rect.width) * disk.size_bytes;
    return Math.round(raw / GIB) * GIB;
  }

  function onDragMove(event) {
    if (!drag || event.pointerId !== drag.pointerId) return;
    const delta = dragDelta(event.clientX);
    if (delta !== 0) drag.moved = true;
    const previewState = { ...drag.startState };
    const preview = resizeBoundary(disk, previewState, drag.id, delta);
    drag.previewState = previewState;
    drag.preview = preview;
    copyLayout(state, previewState);
    paint(preview);
  }

  function endDrag(event) {
    if (!drag || event.pointerId !== drag.pointerId) return;
    const finished = drag;
    cleanupDrag();
    if (!finished.moved || !finished.preview) return;
    onCommit?.(state, finished.preview);
    announce(t('disk.timeline.announce', 'Layout updated: system {root}, free {free}', {
      root: compact(finished.preview.sizes.root, formatBytes),
      free: compact(freeAfterBytes(), formatBytes),
    }));
  }

  function cancelDrag() {
    if (!drag) return;
    const original = { ...drag.startState };
    copyLayout(state, original);
    cleanupDrag();
    paint(buildPlanModel(disk, state));
  }

  function onDragKey(event) {
    if (event.key === 'Escape') {
      event.preventDefault();
      cancelDrag();
    }
  }

  function cleanupDrag() {
    if (drag?.handle) {
      drag.handle.releasePointerCapture?.(drag.pointerId);
      drag.handle.classList.remove('is-dragging');
    }
    window.removeEventListener('pointermove', onDragMove);
    window.removeEventListener('pointerup', endDrag);
    window.removeEventListener('pointercancel', cancelDrag);
    window.removeEventListener('keydown', onDragKey);
    drag = null;
  }

  paint(model);
  return wrap;
}
