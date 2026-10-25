import {
  ESP_BYTES, GIB, MIB, MIN_ROOT_BYTES,
  MODE_ERASE, MODE_EXISTING, MODE_MANUAL,
  buildPlanModel, resizeBoundary, rootCandidates, summaryFor,
} from './timeline-model.js';

const ZOOM_LEVELS = [1, 2, 4, 8];
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

function tickStep(total, zoom) {
  const ticks = Math.max(4, Math.round(6 * Math.min(zoom, 4)));
  const raw = total / ticks;
  const units = [1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000, 2000, 4000]
    .map((value) => value * GIB);
  return units.find((unit) => unit >= raw) || units[units.length - 1];
}

function plannedLabel(kind, t) {
  if (kind === 'efi') return t('disk.timeline.plannedEfi', 'Startup');
  if (kind === 'swap') return t('disk.timeline.plannedSwap', 'Support memory');
  if (kind === 'home') return t('disk.timeline.plannedHome', 'Personal files');
  return t('disk.timeline.plannedRoot', "PEAK'D! system");
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

export function renderTimelinePanel(options) {
  const {
    disk, state, model: initialModel, t, formatBytes,
    onSelectGap, onSelectPartition, onPreset, onRemovePartition,
    onCommit, onRequestRender,
  } = options;
  const wrap = el('div', 'installer-timeline');
  let zoom = ZOOM_LEVELS.includes(Number(state.timelineZoom))
    ? Number(state.timelineZoom)
    : 1;
  let model = initialModel || buildPlanModel(disk, state);

  const toolbar = el('div', 'installer-timeline-toolbar');
  const toolbarCopy = el('div', 'installer-timeline-toolbar-copy');
  toolbarCopy.append(
    el('span', 'installer-timeline-title', t('disk.timeline.title', 'The disk, from start to end')),
    el('span', 'installer-timeline-hint', t('disk.timeline.dragHint', 'Drag the edges of the new partitions. Existing partitions are locked.')),
  );
  toolbar.appendChild(toolbarCopy);

  const zoomWrap = el('div', 'installer-timeline-zoom');
  zoomWrap.setAttribute('role', 'group');
  zoomWrap.setAttribute('aria-label', t('disk.timeline.zoom', 'Zoom'));
  const zoomButtons = [];
  for (const level of ZOOM_LEVELS) {
    const button = el('button', 'installer-zoom-button', level === 1
      ? t('disk.timeline.fit', 'Fit')
      : `${level}×`);
    button.type = 'button';
    button.addEventListener('click', () => {
      const anchor = scroll.scrollWidth
        ? (scroll.scrollLeft + scroll.clientWidth / 2) / scroll.scrollWidth
        : 0;
      zoom = level;
      state.timelineZoom = level;
      paint(model);
      if (anchor) {
        scroll.scrollLeft = Math.max(0, anchor * scroll.scrollWidth - scroll.clientWidth / 2);
      }
    });
    zoomButtons.push([level, button]);
    zoomWrap.appendChild(button);
  }
  toolbar.appendChild(zoomWrap);
  wrap.appendChild(toolbar);

  const scroll = el('div', 'installer-timeline-scroll');
  scroll.dir = 'ltr';
  const content = el('div', 'installer-timeline-content');
  content.style.width = `${zoom * 100}%`;
  scroll.appendChild(content);
  wrap.appendChild(scroll);

  const ruler = el('div', 'installer-ruler');
  ruler.setAttribute('aria-hidden', 'true');
  content.appendChild(ruler);

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

  const playhead = el('div', 'installer-playhead');
  const playheadLabel = el('span', 'installer-playhead-label', t('disk.timeline.installHere', 'Install here'));
  playhead.appendChild(playheadLabel);
  content.appendChild(playhead);

  const handleLayer = el('div', 'installer-handle-layer');
  afterWrap.appendChild(handleLayer);

  const handles = new Map();
  function addHandle(id) {
    const handle = el('button', 'installer-handle');
    handle.type = 'button';
    handle.dataset.boundary = id;
    handle.setAttribute('role', 'separator');
    handle.setAttribute('aria-orientation', 'vertical');
    handle.tabIndex = 0;
    handle.appendChild(el('span', 'installer-handle-grip'));
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

  const legend = el('div', 'installer-timeline-legend');
  for (const [kind, key, fallback] of [
    ['kept', 'disk.timeline.legendKept', 'Kept: not touched'],
    ['planned', 'disk.timeline.legendPlanned', 'New partitions'],
    ['free', 'disk.timeline.legendFree', 'Free space'],
    ['reused', 'disk.timeline.legendReused', 'Startup partition reused'],
    ['formatted', 'disk.timeline.legendFormatted', 'Will be formatted'],
  ]) {
    const item = el('span', 'installer-timeline-legend-item');
    item.append(el('span', `installer-legend-dot installer-legend-dot--${kind}`), el('span', null, t(key, fallback)));
    legend.appendChild(item);
  }
  wrap.appendChild(legend);

  const summary = el('div', 'installer-timeline-summary');
  const summaryFormatted = el('div', 'installer-summary-line');
  const summaryKept = el('div', 'installer-summary-line');
  const summaryFree = el('div', 'installer-summary-line');
  const summaryErrors = el('div', 'installer-summary-errors');
  summary.append(summaryFormatted, summaryKept, summaryFree, summaryErrors);
  wrap.appendChild(summary);

  if (state.partitionMode !== MODE_EXISTING) {
    const presetRow = el('div', 'installer-preset-row');
    const presetChoices = [
      ['simple', 'disk.preset.simple.title', 'Minimal'],
      ['swap', 'disk.preset.swap.title', 'Recommended'],
      ['home', 'disk.preset.home.title', 'Separate home'],
    ];
    for (const [value, key, fallback] of presetChoices) {
      const button = el('button', `installer-preset${state.layoutPreset === value ? ' is-selected' : ''}`, t(key, fallback));
      button.type = 'button';
      button.setAttribute('aria-pressed', String(state.layoutPreset === value));
      button.addEventListener('click', () => onPreset?.(value));
      presetRow.appendChild(button);
    }
    const reset = el('button', 'installer-preset installer-preset--ghost', t('disk.timeline.reset', 'Reset to recommended'));
    reset.type = 'button';
    reset.addEventListener('click', () => onPreset?.('swap'));
    presetRow.appendChild(reset);
    wrap.appendChild(presetRow);
  }

  const exact = el('details', 'installer-exact');
  if (state.exactOpen) exact.open = true;
  const exactSummary = el('summary', 'installer-exact-summary', t('disk.timeline.exact', 'Exact sizes'));
  exact.appendChild(exactSummary);
  const exactBody = el('div', 'installer-exact-body');
  exact.appendChild(exactBody);
  exact.addEventListener('toggle', () => {
    state.exactOpen = exact.open;
    if (exact.open) {
      buildExactPanel();
      paintExact();
    }
  });
  wrap.appendChild(exact);

  const exactRefs = {};
  buildExactPanel();
  const live = el('div', 'installer-sr-only');
  live.setAttribute('aria-live', 'polite');
  wrap.appendChild(live);

  function announce(message) {
    live.textContent = message;
  }

  function currentGap() {
    return model.gap || (model.gaps || [])[0] || null;
  }

  function buildExactPanel() {
    clear(exactBody);
    exactRefs.swap = null;
    exactRefs.home = null;
    exactRefs.homeSize = null;
    exactRefs.root = null;

    const efiRow = el('div', 'installer-exact-row');
    efiRow.append(
      el('span', 'installer-exact-label', t('disk.timeline.plannedEfi', 'Startup')),
      el('span', 'installer-exact-value', compact(ESP_BYTES, formatBytes)),
    );
    exactBody.appendChild(efiRow);

    const zone = el('div', 'installer-exact-hint');
    if (state.partitionMode === MODE_MANUAL && model.gap) {
      zone.textContent = t('disk.timeline.zoneRange', 'Free space: {start} – {end}', {
        start: compact(model.gap.start_bytes, formatBytes),
        end: compact(model.gap.start_bytes + model.gap.size_bytes, formatBytes),
      });
    } else if (state.partitionMode === MODE_ERASE) {
      zone.textContent = t('disk.timeline.wholeDisk', 'The whole disk will be used.');
    } else if (state.partitionMode === MODE_EXISTING) {
      zone.textContent = t('disk.timeline.existingZone', 'Only the chosen partition will be formatted.');
    }
    exactBody.appendChild(zone);

    if (state.partitionMode === MODE_EXISTING) {
      exactBody.appendChild(el('p', 'installer-exact-hint', t('disk.timeline.existingExact', 'Sizes are managed by the existing partition; no new partition is created.')));
      return;
    }

    const swapRow = el('label', 'installer-exact-row');
    swapRow.appendChild(el('span', 'installer-exact-label', t('disk.timeline.plannedSwap', 'Support memory')));
    const swap = el('select', 'installer-exact-select');
    const maxSwap = Math.max(0, Math.floor((model.sizes.maxSwap || 0) / GIB));
    const values = [...new Set([0, 4, 8, 16, 32, Math.round(model.sizes.swap / GIB)])]
      .filter((value) => value <= maxSwap)
      .sort((a, b) => a - b);
    for (const value of values) {
      const option = el('option', null, value === 0
        ? t('disk.manual.none', 'Not created')
        : `${value} GiB`);
      option.value = String(value);
      swap.appendChild(option);
    }
    swap.value = String(Math.round(model.sizes.swap / GIB));
    swap.addEventListener('change', () => {
      const value = Math.max(0, Number(swap.value) || 0);
      if (state.partitionMode === MODE_MANUAL) {
        state.manualSwapGiB = value;
      } else {
        state.swapGiB = value;
        if (value > 0 && state.layoutPreset === 'simple') state.layoutPreset = 'swap';
      }
      onRequestRender?.();
    });
    exactRefs.swap = swap;
    swapRow.appendChild(swap);
    exactBody.appendChild(swapRow);

    const homeRow = el('label', 'installer-exact-row');
    homeRow.appendChild(el('span', 'installer-exact-label', t('disk.timeline.plannedHome', 'Personal files')));
    const homeWrap = el('span', 'installer-exact-inline');
    const home = el('input');
    home.type = 'checkbox';
    if (state.partitionMode === MODE_MANUAL) {
      home.checked = !!state.manualHome;
    } else {
      home.checked = state.layoutPreset === 'home';
    }
    home.addEventListener('change', () => {
      if (state.partitionMode === MODE_MANUAL) {
        state.manualHome = home.checked;
        if (home.checked && !(Number(state.manualHomeGiB) > 0)) {
          state.manualHomeGiB = Math.max(1, Math.floor((model.sizes.afterSwap / 2) / GIB));
        }
      } else {
        state.layoutPreset = home.checked ? 'home' : (model.sizes.swap > 0 ? 'swap' : 'simple');
        state.homeGiB = 0;
      }
      onRequestRender?.();
    });
    const homeSize = el('input', 'installer-exact-number');
    homeSize.type = 'number';
    homeSize.min = '1';
    homeSize.step = '1';
    homeSize.value = state.partitionMode === MODE_MANUAL
      ? String(Math.max(1, Math.round(model.sizes.home / GIB)))
      : String(Math.max(1, Math.round(model.sizes.home / GIB)));
    homeSize.disabled = !home.checked;
    homeSize.setAttribute('aria-label', t('disk.manual.homeSize', 'Size for personal files in GiB'));
    homeSize.addEventListener('change', () => {
      const value = Math.max(1, Number(homeSize.value) || 1);
      if (state.partitionMode === MODE_MANUAL) state.manualHomeGiB = value;
      else state.homeGiB = value;
      onRequestRender?.();
    });
    exactRefs.home = home;
    exactRefs.homeSize = homeSize;
    homeWrap.append(home, homeSize, el('span', 'installer-exact-suffix', 'GiB'));
    homeRow.appendChild(homeWrap);
    exactBody.appendChild(homeRow);

    if (state.partitionMode === MODE_MANUAL) {
      const rootRow = el('label', 'installer-exact-row');
      rootRow.appendChild(el('span', 'installer-exact-label', t('disk.timeline.plannedRoot', "PEAK'D! system")));
      const root = el('input', 'installer-exact-number');
      root.type = 'number';
      root.min = '20';
      root.step = '1';
      root.placeholder = t('disk.manual.rootAuto', 'Rest of free space');
      root.value = state.manualRootGiB > 0 ? String(state.manualRootGiB) : '';
      root.setAttribute('aria-label', t('disk.manual.rootSize', 'Size for the PEAK’D! system in GiB'));
      root.addEventListener('change', () => {
        state.manualRootGiB = Math.max(0, Number(root.value) || 0);
        onRequestRender?.();
      });
      exactRefs.root = root;
      rootRow.append(root, el('span', 'installer-exact-suffix', 'GiB'));
      exactBody.appendChild(rootRow);
      if (model.remaining > 0) {
        exactBody.appendChild(el('p', 'installer-exact-hint', t('disk.timeline.remaining', 'Unused free space: {size}', {
          size: compact(model.remaining, formatBytes),
        })));
      }
    }
  }

  function segmentClass(segment) {
    if (segment.kind === 'free') return 'installer-clip installer-clip--free';
    if (segment.planned) return `installer-clip installer-clip--planned installer-clip--${segment.kind}`;
    if (segment.selectedRoot) return 'installer-clip installer-clip--formatted';
    if (segment.reused) return 'installer-clip installer-clip--reused';
    if (model.mode === MODE_ERASE && segment.part) return 'installer-clip installer-clip--doomed';
    return `installer-clip installer-clip--kept installer-clip--${segment.kind}`;
  }

  function clipBody(segment) {
    let label = segment.label || '';
    if (segment.planned) label = plannedLabel(segment.kind, t);
    if (segment.kind === 'free') label = t('disk.free', 'Free space');
    if (segment.selectedRoot) label = t('disk.timeline.plannedRoot', "PEAK'D! system");
    if (segment.reused) label = t('disk.timeline.plannedEfi', 'Startup');
    const body = el('span', 'installer-clip-copy');
    body.append(
      el('span', 'installer-clip-label', label),
      el('span', 'installer-clip-size', compact(segment.size, formatBytes)),
    );
    return body;
  }

  function appendSegment(track, segment) {
    const widthPct = pct(segment.size, disk.size_bytes);
    const isGap = segment.kind === 'free';
    const gapRecord = isGap && (model.gaps || []).find((gap) => (
      gap.start_bytes === segment.start && gap.size_bytes === segment.size
    ));
    const canSelectGap = state.partitionMode !== MODE_ERASE
      && gapRecord?.eligible && segment.source === 'probe';
    const selected = isGap && model.gap && segment.start === model.gap.start_bytes;
    let node;
    if (isGap && canSelectGap) {
      node = el('button', segmentClass(segment));
      node.type = 'button';
      node.setAttribute('aria-label', `${t('disk.timeline.gap', 'Free space')} ${compact(segment.size, formatBytes)} — ${t('disk.timeline.installHere', 'Install here')}`);
      node.addEventListener('click', () => onSelectGap?.(segment.gap || {
        start_bytes: segment.start,
        size_bytes: segment.size,
      }));
    } else if (segment.part && state.partitionMode !== MODE_ERASE
      && rootCandidates(disk).some((part) => part.path === segment.part.path)) {
      node = el('button', segmentClass(segment));
      node.type = 'button';
      node.setAttribute('aria-label', `${segment.label} — ${t('disk.existing.pickOne', 'Install PEAK’D! in this partition')}`);
      node.addEventListener('click', () => onSelectPartition?.(segment.part));
    } else {
      node = el('div', segmentClass(segment));
    }
    node.style.left = `${pct(segment.start, disk.size_bytes)}%`;
    node.style.width = `${widthPct}%`;
    if (widthPct < 4) node.classList.add('is-tiny');
    if (selected) node.classList.add('is-selected');
    if (isGap) {
      if (canSelectGap) {
        node.appendChild(el('span', 'installer-gap-action', t('disk.timeline.installHere', 'Install here')));
      }
      node.appendChild(clipBody(segment));
      if (selected) {
        node.setAttribute('aria-current', 'true');
      }
    } else {
      node.appendChild(clipBody(segment));
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
    const detail = segment.part?.path || (isGap ? t('disk.freeDetail', 'No partition table entry') : '');
    node.title = `${clipTitle(segment)}${detail ? ` · ${detail}` : ''}`;
    track.appendChild(node);
  }

  function clipTitle(segment) {
    if (segment.planned) return `${plannedLabel(segment.kind, t)} · ${compact(segment.size, formatBytes)}`;
    if (segment.kind === 'free') return `${t('disk.free', 'Free space')} · ${compact(segment.size, formatBytes)}`;
    return `${segment.label || segment.part?.path || ''} · ${compact(segment.size, formatBytes)}`;
  }

  function paint(nextModel) {
    model = nextModel;
    content.style.width = `${zoom * 100}%`;
    clear(ruler);
    const step = tickStep(disk.size_bytes, zoom);
    for (let offset = 0; offset <= disk.size_bytes; offset += step) {
      const tick = el('span', 'installer-ruler-tick');
      tick.style.left = `${pct(offset, disk.size_bytes)}%`;
      tick.append(
        el('span', 'installer-ruler-mark'),
        el('span', 'installer-ruler-label', offset === 0 ? '0' : compact(offset, formatBytes)),
      );
      ruler.appendChild(tick);
    }

    clear(nowTrack);
    for (const segment of model.now) appendSegment(nowTrack, segment);
    clear(afterTrack);
    for (const segment of model.after) appendSegment(afterTrack, segment);

    const gap = currentGap();
    if (gap && (state.partitionMode === MODE_MANUAL || state.partitionMode === MODE_ERASE)) {
      playhead.style.display = '';
      playhead.style.left = `${pct(state.partitionMode === MODE_ERASE ? MIB : gap.start_bytes, disk.size_bytes)}%`;
    } else {
      playhead.style.display = 'none';
    }

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
    }

    for (const [level, button] of zoomButtons) {
      button.classList.toggle('is-selected', level === zoom);
      button.setAttribute('aria-pressed', String(level === zoom));
    }

    paintSummary();
    paintExact();
  }

  function paintSummary() {
    const summaryData = summaryFor(disk, state);
    const formatted = summaryData.formattedPaths;
    summaryFormatted.textContent = formatted.length
      ? `${t('disk.summary.formatted', 'Will be formatted')}: ${formatted.join(', ')}`
      : t('disk.summary.nothingTouched', 'No existing partition will be touched');
    summaryKept.textContent = `${t('disk.summary.kept', 'Kept partitions')}: ${summaryData.keptCount} · ${compact(summaryData.keptBytes, formatBytes)}`;
    const freeText = state.partitionMode === MODE_ERASE
      ? t('disk.summary.erased', 'The whole disk is used by PEAK’D!.')
      : state.partitionMode === MODE_EXISTING
        ? t('disk.timeline.existingZone', 'Only the chosen partition will be formatted.')
        : `${t('disk.summary.freeAfter', 'Free space after install')}: ${compact(freeAfterBytes(), formatBytes)}`;
    summaryFree.textContent = freeText;
    clear(summaryErrors);
    for (const error of model.errors) {
      summaryErrors.appendChild(el('p', 'installer-summary-error installer-inline-error', errorLabel(error)));
    }
    if (model.approx) {
      summaryErrors.appendChild(el('p', 'installer-summary-warn', t('disk.timeline.noOffsets', 'Partition positions are approximate; refresh the disks before changing the layout.')));
    }
  }

  function freeAfterBytes() {
    return Math.max(0, model.remaining || 0);
  }

  function errorLabel(code) {
    const values = {
      noDisk: t('disk.noDisk', 'No disk selected'),
      noFree: t('disk.manual.noFree', 'No usable free space found on this disk'),
      noOffsets: t('disk.timeline.noOffsets', 'Partition positions are approximate; refresh the disks before changing the layout.'),
      gapTooSmall: t('disk.timeline.gapTooSmall', 'The free space is too small for PEAK’D!.'),
      swapTooBig: t('disk.timeline.swapTooBig', 'The support memory does not fit in this free space.'),
      homeTooBig: t('disk.timeline.homeTooBig', 'The personal-files partition does not fit in this free space.'),
      rootTooBig: t('disk.timeline.rootTooBig', 'The system does not fit in this free space.'),
      rootSmall: t('disk.manual.rootSmall', 'Leave at least 20 GiB for the system.'),
      noRoot: t('disk.existing.noRoot', 'Select an unmounted ext4 partition to use as root'),
      noEsp: t('disk.existing.noEsp', 'No EFI system partition found on this disk'),
    };
    return values[code] || code;
  }

  function paintExact() {
    if (exactRefs.swap) {
      const value = String(Math.round(model.sizes.swap / GIB));
      const options = Array.from(exactRefs.swap.children || []);
      const known = options.some((option) => option.value === value);
      if (!known && exact.open) {
        const option = el('option', null, `${value} GiB`);
        option.value = value;
        exactRefs.swap.appendChild(option);
      }
      if ((known || exact.open) && document.activeElement !== exactRefs.swap) {
        exactRefs.swap.value = value;
      }
    }
    if (exactRefs.home) {
      if (document.activeElement !== exactRefs.home) {
        exactRefs.home.checked = state.partitionMode === MODE_MANUAL
          ? !!state.manualHome
          : state.layoutPreset === 'home';
      }
    }
    if (exactRefs.homeSize) {
      exactRefs.homeSize.disabled = !exactRefs.home?.checked;
      if (document.activeElement !== exactRefs.homeSize) {
        exactRefs.homeSize.value = String(Math.max(1, Math.round(model.sizes.home / GIB)));
      }
    }
    if (exactRefs.root && document.activeElement !== exactRefs.root) {
      exactRefs.root.value = state.manualRootGiB > 0 ? String(state.manualRootGiB) : '';
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
