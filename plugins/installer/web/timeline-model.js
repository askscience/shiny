export const MIB = 1024 ** 2;
export const GIB = 1024 ** 3;
export const MIN_DISK_BYTES = 8 * GIB;
export const MIN_ROOT_BYTES = 20 * GIB;
export const ESP_BYTES = 512 * MIB;
export const PLACEMENT_GUARD = 1 * MIB;

export const MODE_ERASE = 'erase';
export const MODE_EXISTING = 'existing';
export const MODE_MANUAL = 'manual';

const number = (value) => {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : 0;
};

const clamp = (value, min, max) => Math.min(Math.max(value, min), max);
const alignDown = (bytes, unit = MIB) => Math.floor(number(bytes) / unit) * unit;
const bytesToGiB = (bytes) => Math.max(0, Math.round(number(bytes) / GIB));
const fromGiB = (gib) => Math.max(0, number(gib) * GIB);

export function classifyPartition(part) {
  const fstype = String(part?.fstype || '').toLowerCase();
  const mounts = part?.mountpoints || [];
  if (fstype.includes('swap')) return 'swap';
  if (mounts.includes('/boot/efi') || fstype === 'vfat' || fstype === 'fat32') return 'efi';
  if (mounts.includes('/home')) return 'home';
  if (mounts.includes('/')) return 'root';
  return 'foreign';
}

export function isEsp(part) {
  const fstype = String(part?.fstype || '').toLowerCase();
  return fstype === 'vfat'
    || fstype === 'fat32'
    || (part?.partflags || []).includes('esp')
    || String(part?.parttypename || '').toLowerCase().includes('efi');
}

export function isSwap(part) {
  const fstype = String(part?.fstype || '').toLowerCase();
  return fstype.includes('swap')
    || (part?.mountpoints || []).includes('[SWAP]')
    || String(part?.parttypename || '').toLowerCase().includes('swap');
}

export function isForeignPartition(part) {
  const fstype = String(part?.fstype || '').toLowerCase();
  const typename = String(part?.parttypename || '').toLowerCase();
  return (part?.holders || []).length > 0
    || ['apfs', 'hfs', 'hfsplus', 'ntfs', 'bitlocker', 'crypto_luks', 'lvm2_member', 'linux_raid_member'].includes(fstype)
    || typename.includes('apfs')
    || typename.includes('hfs')
    || typename.includes('bitlocker')
    || typename.includes('lvm')
    || typename.includes('raid');
}

export function diskPaths(disk) {
  const paths = [];
  const walk = (node) => {
    if (node?.path) paths.push(node.path);
    for (const child of node?.children || []) walk(child);
  };
  walk(disk);
  return paths;
}

export function diskHasMounts(disk) {
  return (disk?.mountpoints?.length || 0) > 0
    || (disk?.children || []).some(diskHasMounts);
}

export function diskIsLive(disk, liveMedium = null) {
  return !!liveMedium && diskPaths(disk).includes(liveMedium);
}

export function diskReasons(disk, mode = MODE_ERASE, liveMedium = null) {
  if (!disk) return ['missing'];
  const reasons = [];
  if (disk.removable) reasons.push('removable');
  if (disk.read_only) reasons.push('readOnly');
  if (disk.is_loop) reasons.push('loop');
  if (disk.is_rom) reasons.push('optical');
  if (mode === MODE_ERASE && diskHasMounts(disk)) reasons.push('mounted');
  if (diskIsLive(disk, liveMedium)) reasons.push('live');
  if (!disk.serial) reasons.push('serial');
  if (number(disk.size_bytes) < MIN_DISK_BYTES) reasons.push('small');
  if (disk.pttype && disk.pttype !== 'gpt') reasons.push('table');
  return reasons;
}

export function rootCandidates(disk) {
  return (disk?.children || []).filter((part) => {
    if (isEsp(part) || isSwap(part) || isForeignPartition(part)) return false;
    if ((part.mountpoints || []).length > 0) return false;
    if (part.fstype && part.fstype !== 'ext4') return false;
    return number(part.size_bytes) >= MIN_ROOT_BYTES;
  });
}

export function findEsp(disk) {
  return (disk?.children || []).find(isEsp) || null;
}

export function usableGapBytes(gap) {
  const size = number(gap?.size_bytes ?? gap?.size);
  return alignDown(Math.max(0, size - PLACEMENT_GUARD));
}

export function usableDiskBytes(totalBytes) {
  return alignDown(Math.max(0, number(totalBytes) - MIB));
}

function freeSegment(gap, source = 'probe') {
  return {
    kind: 'free',
    start: number(gap.start_bytes ?? gap.start),
    size: number(gap.size_bytes ?? gap.size),
    source,
    gap: {
      start_bytes: number(gap.start_bytes ?? gap.start),
      size_bytes: number(gap.size_bytes ?? gap.size),
    },
    locked: true,
  };
}

export function diskSpans(disk) {
  const total = number(disk?.size_bytes);
  const children = (disk?.children || []).filter((child) => number(child.size_bytes) > 0);
  const hasOffsets = children.length > 0 && children.every((child) => number(child.start_bytes) > 0);
  const ordered = hasOffsets
    ? [...children].sort((a, b) => number(a.start_bytes) - number(b.start_bytes))
    : children;
  let cursor = MIB;
  const segments = [];

  for (const child of ordered) {
    const size = number(child.size_bytes);
    const start = hasOffsets ? number(child.start_bytes) : cursor;
    segments.push({
      kind: classifyPartition(child),
      start,
      size,
      part: child,
      label: child.mountpoints?.[0] || child.fstype || child.name || child.path,
      locked: true,
      approx: !hasOffsets,
    });
    cursor = start + size;
  }

  const probeGaps = (disk?.free_extents || []).filter((gap) => number(gap.size_bytes) > 0);
  if (probeGaps.length) {
    for (const gap of probeGaps) {
      const start = clamp(number(gap.start_bytes), 0, total);
      const size = Math.min(number(gap.size_bytes), Math.max(0, total - start));
      if (size > 0) segments.push(freeSegment({ start_bytes: start, size_bytes: size }, 'probe'));
    }
  } else {
    const size = Math.max(0, total - cursor);
    if (size > 0) segments.push(freeSegment({ start_bytes: cursor, size_bytes: size }, 'approx'));
  }

  return segments.sort((a, b) => a.start - b.start);
}

export function diskGaps(disk) {
  const total = number(disk?.size_bytes);
  const spans = (disk?.free_extents || [])
    .map((gap) => {
      const start = clamp(number(gap.start_bytes), 0, total);
      const size = Math.min(number(gap.size_bytes), Math.max(0, total - start));
      return { start_bytes: start, size_bytes: size };
    })
    .filter((gap) => gap.size_bytes > 0)
    .sort((a, b) => a.start_bytes - b.start_bytes);

  if (spans.length) return spans.map((gap) => gapRecord(gap, 'probe'));
  return diskSpans(disk)
    .filter((segment) => segment.kind === 'free')
    .map((segment) => gapRecord(segment.gap, segment.source));
}

function gapRecord(gap, source) {
  const usableBytes = usableGapBytes(gap);
  const minimum = ESP_BYTES + MIN_ROOT_BYTES;
  let reason = '';
  if (source !== 'probe') reason = 'noOffsets';
  else if (usableBytes < minimum) reason = 'tooSmall';
  return {
    start_bytes: number(gap.start_bytes),
    size_bytes: number(gap.size_bytes),
    usableBytes,
    source,
    eligible: !reason,
    reason,
  };
}

export function selectedGap(disk, state) {
  const gaps = diskGaps(disk);
  if (!gaps.length) return null;
  const wanted = state?.selectedExtent;
  if (wanted) {
    const found = gaps.find((gap) => gap.start_bytes === number(wanted.start_bytes));
    if (found) return found;
  }
  const eligible = gaps
    .filter((gap) => gap.eligible)
    .sort((a, b) => b.usableBytes - a.usableBytes);
  if (eligible.length) return eligible[0];
  return gaps.slice().sort((a, b) => b.size_bytes - a.size_bytes)[0];
}

export function presetBytes(mode, preset, rawBytes) {
  const usable = mode === MODE_ERASE
    ? usableDiskBytes(rawBytes)
    : usableGapBytes({ size_bytes: rawBytes });
  const efi = Math.min(ESP_BYTES, usable);
  const available = Math.max(0, usable - efi);
  let swap = 0;
  if (preset !== 'simple') {
    swap = alignDown(Math.min(8 * GIB, Math.max(0, available - MIN_ROOT_BYTES)));
  }
  let home = 0;
  if (preset === 'home') {
    const afterSwap = Math.max(0, available - swap);
    const maxHome = Math.max(0, afterSwap - MIN_ROOT_BYTES);
    home = alignDown(Math.min(Math.floor(afterSwap / 2), maxHome));
  }
  const root = Math.max(0, available - swap - home);
  return { efi, swap, home, root, usable };
}

export function applyPreset(state, mode, preset, rawBytes) {
  const sizes = presetBytes(mode, preset, rawBytes);
  state.layoutPreset = preset;
  if (mode === MODE_MANUAL) {
    state.manualSwapGiB = bytesToGiB(sizes.swap);
    state.manualHome = sizes.home > 0;
    state.manualHomeGiB = bytesToGiB(sizes.home);
    state.manualRootGiB = 0;
  } else {
    state.swapGiB = bytesToGiB(sizes.swap);
    state.homeGiB = 0;
  }
  return state;
}

function plannedSegment(kind, start, size) {
  return {
    kind,
    role: kind === 'efi' ? 'esp' : kind,
    start,
    size,
    planned: true,
    locked: false,
    labelKind: kind,
  };
}

function buildEraseSizes(disk, state) {
  const total = number(disk?.size_bytes);
  const usable = usableDiskBytes(total);
  const efi = Math.min(ESP_BYTES, usable);
  let maxSwap = Math.max(0, usable - efi - MIN_ROOT_BYTES);
  let swap = state.layoutPreset === 'simple'
    ? 0
    : alignDown(clamp(fromGiB(state.swapGiB), 0, maxSwap));
  const afterSwap = Math.max(0, usable - efi - swap);
  let home = 0;
  if (state.layoutPreset === 'home') {
    const maxHome = Math.max(0, afterSwap - MIN_ROOT_BYTES);
    home = state.homeGiB > 0
      ? alignDown(clamp(fromGiB(state.homeGiB), 0, maxHome))
      : alignDown(Math.min(Math.floor(afterSwap / 2), maxHome));
  }
  const root = Math.max(0, afterSwap - home);
  maxSwap = Math.max(0, usable - efi - home - MIN_ROOT_BYTES);
  const maxHome = Math.max(0, afterSwap - MIN_ROOT_BYTES);
  return { usable, efi, swap, home, root, remaining: 0, maxSwap, maxHome };
}

function buildManualSizes(disk, state, gap) {
  const usable = gap ? gap.usableBytes : 0;
  const efi = Math.min(ESP_BYTES, usable);
  const maxSwap = Math.max(0, usable - efi - MIN_ROOT_BYTES);
  const swapRequested = fromGiB(state.manualSwapGiB);
  const swap = alignDown(clamp(swapRequested, 0, maxSwap));
  const afterSwap = Math.max(0, usable - efi - swap);
  const maxHome = Math.max(0, afterSwap - MIN_ROOT_BYTES);
  const homeEnabled = !!state.manualHome;
  const homeRequested = fromGiB(state.manualHomeGiB);
  let home = 0;
  if (homeEnabled) {
    home = homeRequested > 0 ? homeRequested : GIB;
    home = alignDown(clamp(home, Math.min(GIB, maxHome), maxHome));
  }
  const rootAuto = Math.max(0, afterSwap - home);
  const rootRequested = state.manualRootGiB > 0
    ? alignDown(fromGiB(state.manualRootGiB))
    : 0;
  const root = rootRequested > 0 ? Math.min(rootRequested, rootAuto) : rootAuto;
  const remaining = Math.max(0, rootAuto - root);
  return {
    usable,
    efi,
    swap,
    home,
    root,
    rootAuto,
    rootRequested,
    afterSwap,
    remaining,
    maxSwap,
    maxHome,
    swapRequested,
    homeRequested,
    homeEnabled,
  };
}

export function buildPlanModel(disk, state) {
  const mode = state?.partitionMode || MODE_ERASE;
  const gaps = diskGaps(disk);
  const gap = mode === MODE_MANUAL ? selectedGap(disk, state) : null;
  const sizes = mode === MODE_MANUAL
    ? buildManualSizes(disk, state, gap)
    : buildEraseSizes(disk, state);
  const errors = [];
  let now = diskSpans(disk);
  let after = [];
  let formatted = [];

  if (!disk) {
    errors.push('noDisk');
  } else if (mode === MODE_MANUAL) {
    if (!gap) errors.push('noFree');
    else if (gap.source !== 'probe') errors.push('noOffsets');
    else if (sizes.usable < ESP_BYTES + MIN_ROOT_BYTES) errors.push('gapTooSmall');
    if (sizes.swapRequested > sizes.maxSwap + MIB) errors.push('swapTooBig');
    if (sizes.homeEnabled && sizes.homeRequested > sizes.maxHome + MIB) errors.push('homeTooBig');
    if (sizes.rootRequested > 0 && sizes.rootRequested > sizes.rootAuto) errors.push('rootTooBig');
    if (sizes.root < MIN_ROOT_BYTES) errors.push('rootSmall');
  } else if (mode === MODE_ERASE) {
    if (sizes.usable < ESP_BYTES + MIN_ROOT_BYTES) errors.push('gapTooSmall');
    if (sizes.root < MIN_ROOT_BYTES) errors.push('rootSmall');
    formatted = (disk?.children || []).filter((child) => number(child.size_bytes) > 0);
  } else {
    const root = (disk?.children || []).find((part) => part.path === state?.existingRootPath);
    const esp = findEsp(disk);
    if (!root) errors.push('noRoot');
    if (!esp) errors.push('noEsp');
    if (root && number(root.size_bytes) < MIN_ROOT_BYTES) errors.push('rootSmall');
    if (root) formatted = [root];
    after = now.map((segment) => {
      if (segment.part?.path === root?.path) {
        return { ...segment, kind: 'root', selectedRoot: true, locked: false };
      }
      if (segment.part && isEsp(segment.part)) {
        return { ...segment, reused: true };
      }
      return segment;
    });
  }

  if (mode === MODE_ERASE) {
    let start = MIB;
    after = [];
    for (const [kind, size] of [['efi', sizes.efi], ['swap', sizes.swap], ['home', sizes.home], ['root', sizes.root]]) {
      if (size > 0) {
        after.push(plannedSegment(kind, start, size));
        start += size;
      }
    }
  } else if (mode === MODE_MANUAL && !gap) {
    after = now;
  } else if (mode === MODE_MANUAL && gap) {
    let start = gap.start_bytes;
    const planned = [];
    for (const [kind, size] of [['efi', sizes.efi], ['swap', sizes.swap], ['home', sizes.home], ['root', sizes.root]]) {
      if (size > 0) {
        planned.push(plannedSegment(kind, start, size));
        start += size;
      }
    }
    after = now.filter((segment) => !(
      segment.kind === 'free' && segment.start === gap.start_bytes
    ));
    after.push(...planned);
    const rawLeft = Math.max(0, gap.size_bytes - (start - gap.start_bytes));
    if (rawLeft > 0) {
      after.push(freeSegment({ start_bytes: start, size_bytes: rawLeft }, 'probe'));
    }
    after.sort((a, b) => a.start - b.start);
  }

  const kept = (disk?.children || []).filter((child) => number(child.size_bytes) > 0
    && !formatted.some((part) => part.path === child.path));
  const approx = now.some((segment) => segment.approx)
    || gaps.some((gapItem) => gapItem.source !== 'probe');

  return {
    disk,
    mode,
    gap,
    gaps,
    now,
    after,
    sizes,
    errors,
    formatted,
    kept,
    approx,
    remaining: Math.max(0, sizes.remaining || 0),
  };
}

export function layoutSpecsForPlan(disk, state) {
  const model = buildPlanModel(disk, state);
  if (model.mode === MODE_EXISTING) return [];
  const spec = (role, size, fs, mount, flags = []) => ({
    role,
    size_bytes: size,
    size_percent: null,
    fs,
    mount,
    label: null,
    flags,
    wipe: true,
  });
  const specs = [spec('esp', model.sizes.efi, 'vfat', '/boot/efi', ['esp'])];
  if (model.sizes.swap > 0) specs.push(spec('swap', model.sizes.swap, 'swap', null));
  if (model.sizes.home > 0) specs.push(spec('home', model.sizes.home, 'ext4', '/home'));
  specs.push(spec('root', model.sizes.root, 'ext4', '/'));
  return specs;
}

export function placementForPlan(disk, state) {
  if (state?.partitionMode !== MODE_MANUAL) return null;
  const gap = selectedGap(disk, state);
  if (!gap || gap.source !== 'probe') return null;
  return { start_bytes: gap.start_bytes, size_bytes: gap.size_bytes };
}

export function resizeBoundary(disk, state, boundaryId, deltaBytes) {
  const model = buildPlanModel(disk, state);
  const { usable, efi, swap, home, root } = model.sizes;
  const delta = number(deltaBytes);

  if (state.partitionMode === MODE_MANUAL) {
    if (boundaryId === 'swap-root') {
      const maxSwap = Math.max(0, usable - efi - home - MIN_ROOT_BYTES);
      state.manualSwapGiB = bytesToGiB(clamp(swap + delta, 0, maxSwap));
    } else if (boundaryId === 'home-root' && state.manualHome) {
      const maxHome = Math.max(0, usable - efi - swap - MIN_ROOT_BYTES);
      state.manualHomeGiB = bytesToGiB(clamp(home + delta, Math.min(GIB, maxHome), maxHome));
    } else if (boundaryId === 'root-end') {
      const maxRoot = Math.max(0, usable - efi - swap - home);
      const next = clamp(root + delta, MIN_ROOT_BYTES, maxRoot);
      state.manualRootGiB = next >= maxRoot - MIB ? 0 : Math.max(20, bytesToGiB(next));
    }
  } else if (state.partitionMode === MODE_ERASE) {
    if (boundaryId === 'swap-root') {
      const maxSwap = Math.max(0, usable - efi - home - MIN_ROOT_BYTES);
      state.swapGiB = bytesToGiB(clamp(swap + delta, 0, maxSwap));
    } else if (boundaryId === 'home-root' && state.layoutPreset === 'home') {
      const maxHome = Math.max(0, usable - efi - swap - MIN_ROOT_BYTES);
      state.homeGiB = bytesToGiB(clamp(home + delta, Math.min(GIB, maxHome), maxHome));
    }
  }
  return buildPlanModel(disk, state);
}

export function summaryFor(disk, state) {
  const model = buildPlanModel(disk, state);
  return {
    mode: model.mode,
    formatted: model.formatted,
    formattedPaths: model.formatted.map((part) => part.path),
    keptCount: model.kept.length,
    keptBytes: model.kept.reduce((sum, part) => sum + number(part.size_bytes), 0),
    freeAfter: model.remaining,
    approx: model.approx,
    errors: model.errors,
  };
}

function diskRank(a, b) {
  const removable = Number(!!a.removable) - Number(!!b.removable);
  if (removable) return removable;
  const usb = Number(String(a.tran || '').toLowerCase() === 'usb')
    - Number(String(b.tran || '').toLowerCase() === 'usb');
  if (usb) return usb;
  return number(b.size_bytes) - number(a.size_bytes);
}

export function chooseDefaults(probe, options = {}) {
  const liveMedium = options.liveMedium ?? probe?.live_medium ?? null;
  const disks = [...(probe?.disks || [])].sort(diskRank);
  const allowed = (disk, mode) => diskReasons(disk, mode, liveMedium).length === 0;

  for (const disk of disks) {
    if (!allowed(disk, MODE_MANUAL)) continue;
    const gaps = diskGaps(disk)
      .filter((gap) => gap.eligible)
      .sort((a, b) => b.usableBytes - a.usableBytes);
    if (!gaps.length) continue;
    const gap = gaps[0];
    const withSwap = gap.usableBytes >= ESP_BYTES + 8 * GIB + MIN_ROOT_BYTES;
    return {
      diskPath: disk.path,
      mode: MODE_MANUAL,
      preset: withSwap ? 'swap' : 'simple',
      extent: { start_bytes: gap.start_bytes, size_bytes: gap.size_bytes },
      rootPath: '',
      espPath: '',
      reasons: [],
    };
  }

  for (const disk of disks) {
    if ((disk.children || []).length) continue;
    if (!allowed(disk, MODE_ERASE)) continue;
    if (usableDiskBytes(disk.size_bytes) < ESP_BYTES + MIN_ROOT_BYTES) continue;
    return {
      diskPath: disk.path,
      mode: MODE_ERASE,
      preset: 'swap',
      extent: null,
      rootPath: '',
      espPath: '',
      reasons: [],
    };
  }

  for (const disk of disks) {
    if (!allowed(disk, MODE_EXISTING)) continue;
    if (!findEsp(disk)) continue;
    const roots = rootCandidates(disk);
    if (!roots.length) continue;
    const blank = roots.find((part) => !String(part.fstype || '').trim());
    return {
      diskPath: disk.path,
      mode: MODE_EXISTING,
      preset: 'simple',
      extent: null,
      rootPath: blank?.path || '',
      espPath: findEsp(disk)?.path || '',
      reasons: [],
    };
  }

  const first = disks[0];
  return {
    diskPath: first?.path || '',
    mode: MODE_MANUAL,
    preset: 'swap',
    extent: null,
    rootPath: '',
    espPath: '',
    reasons: first ? diskReasons(first, MODE_MANUAL, liveMedium) : ['missing'],
  };
}
