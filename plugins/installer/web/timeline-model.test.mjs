import assert from 'node:assert/strict';
import {
  ESP_BYTES, GIB, MIB, MIN_ROOT_BYTES,
  MODE_ERASE, MODE_EXISTING, MODE_MANUAL,
  applyPreset, buildPlanModel, chooseDefaults, diskGaps, diskReasons,
  diskSpans, layoutSpecsForPlan, placementForPlan, presetBytes,
  resizeBoundary, selectedGap, summaryFor,
} from './timeline-model.js';

let checks = 0;
function check(name, fn) {
  fn();
  checks += 1;
  console.log(`  ok  ${name}`);
}

const freeStart = 300 * GIB + MIB;
const disk = {
  name: 'nvme0n1',
  path: '/dev/nvme0n1',
  kind: 'disk',
  size_bytes: 1000 * GIB,
  serial: 'S1',
  model: 'Synthetic SSD',
  tran: 'nvme',
  pttype: 'gpt',
  removable: false,
  read_only: false,
  mountpoints: [],
  children: [
    {
      name: 'nvme0n1p1',
      path: '/dev/nvme0n1p1',
      start_bytes: MIB,
      size_bytes: 200 * GIB,
      fstype: 'ntfs',
      parttypename: 'Microsoft basic data',
      mountpoints: [],
      holders: [],
    },
    {
      name: 'nvme0n1p2',
      path: '/dev/nvme0n1p2',
      start_bytes: 200 * GIB + MIB,
      size_bytes: 100 * GIB,
      fstype: 'ext4',
      mountpoints: [],
      holders: [],
    },
  ],
  free_extents: [{ start_bytes: freeStart, size_bytes: 700 * GIB - MIB }],
};

function manualState(overrides = {}) {
  return {
    partitionMode: MODE_MANUAL,
    layoutPreset: 'swap',
    swapGiB: 8,
    homeGiB: 0,
    manualSwapGiB: 8,
    manualHome: false,
    manualHomeGiB: 0,
    manualRootGiB: 0,
    selectedExtent: { start_bytes: freeStart, size_bytes: 700 * GIB - MIB },
    selectedDiskPath: disk.path,
    existingRootPath: '',
    ...overrides,
  };
}

check('spans keep the real disk positions', () => {
  const spans = diskSpans(disk);
  assert.equal(spans.length, 3);
  assert.equal(spans[0].start, MIB);
  assert.equal(spans[1].start, 200 * GIB + MIB);
  assert.equal(spans[2].kind, 'free');
  assert.equal(spans[2].start, freeStart);
  assert.equal(spans[2].source, 'probe');
});

check('free gaps are usable and selectable', () => {
  const gaps = diskGaps(disk);
  assert.equal(gaps.length, 1);
  assert.equal(gaps[0].eligible, true);
  assert.equal(gaps[0].source, 'probe');
  const state = manualState();
  assert.equal(selectedGap(disk, state).start_bytes, freeStart);
});

check('the default is the largest usable gap with the recommended preset', () => {
  const defaults = chooseDefaults({ disks: [disk], live_medium: null });
  assert.equal(defaults.mode, MODE_MANUAL);
  assert.equal(defaults.preset, 'swap');
  assert.equal(defaults.extent.start_bytes, freeStart);
});

check('manual sizes fill the chosen gap and leave the minimum root', () => {
  const model = buildPlanModel(disk, manualState());
  assert.equal(model.errors.length, 0);
  assert.equal(model.sizes.efi, ESP_BYTES);
  assert.equal(model.sizes.swap, 8 * GIB);
  assert.ok(model.sizes.root >= MIN_ROOT_BYTES);
  assert.equal(model.sizes.remaining, 0);
  assert.ok(model.sizes.root < 700 * GIB);
});

check('the plan specs and placement match the chosen gap', () => {
  const state = manualState();
  const specs = layoutSpecsForPlan(disk, state);
  assert.deepEqual(specs.map((spec) => spec.role), ['esp', 'swap', 'root']);
  const placement = placementForPlan(disk, state);
  assert.equal(placement.start_bytes, freeStart);
  assert.equal(placement.size_bytes, 700 * GIB - MIB);
});

check('dragging swap grows swap and shrinks root', () => {
  const state = manualState();
  const before = buildPlanModel(disk, state);
  const after = resizeBoundary(disk, state, 'swap-root', 10 * GIB);
  assert.equal(state.manualSwapGiB, 18);
  assert.equal(after.sizes.root, before.sizes.root - 10 * GIB);
});

check('the root end leaves unallocated space in manual mode', () => {
  const state = manualState();
  const after = resizeBoundary(disk, state, 'root-end', -50 * GIB);
  assert.equal(state.manualRootGiB, Math.round((after.sizes.root) / GIB));
  assert.ok(after.remaining >= 49 * GIB);
  assert.ok(after.sizes.root >= MIN_ROOT_BYTES);
});

check('a requested root below the minimum is clamped', () => {
  const state = manualState({ manualSwapGiB: 1000 });
  const model = buildPlanModel(disk, state);
  assert.ok(model.sizes.root >= MIN_ROOT_BYTES);
  assert.ok(model.errors.includes('swapTooBig'));
});

check('manual sizes expose the post-swap space used by the exact panel', () => {
  const model = buildPlanModel(disk, manualState());
  assert.ok(model.sizes.afterSwap >= model.sizes.root);
  assert.ok(model.sizes.afterSwap > 0);
  assert.ok(Number.isFinite(model.sizes.afterSwap));
});

check('presets apply the exact sizes to manual state', () => {
  const state = manualState();
  applyPreset(state, MODE_MANUAL, 'home', 700 * GIB - MIB);
  const model = buildPlanModel(disk, state);
  assert.equal(state.manualHome, true);
  assert.ok(model.sizes.home > 1 * GIB);
  assert.ok(model.sizes.root >= MIN_ROOT_BYTES);
});

check('erase presets use the whole disk', () => {
  const sizes = presetBytes(MODE_ERASE, 'swap', 1000 * GIB);
  assert.equal(sizes.efi, ESP_BYTES);
  assert.equal(sizes.swap, 8 * GIB);
  assert.ok(sizes.root > 900 * GIB);
});

check('an empty disk defaults to erase', () => {
  const empty = {
    name: 'sda', path: '/dev/sda', size_bytes: 120 * GIB,
    serial: 'E1', pttype: 'gpt', removable: false, read_only: false,
    mountpoints: [], children: [], free_extents: [],
  };
  const defaults = chooseDefaults({ disks: [empty], live_medium: null });
  assert.equal(defaults.mode, MODE_ERASE);
  assert.equal(defaults.preset, 'swap');
});

check('a full Windows disk is never selected destructively by default', () => {
  const windows = {
    name: 'sdb', path: '/dev/sdb', size_bytes: 500 * GIB,
    serial: 'W1', pttype: 'gpt', removable: false, read_only: false,
    mountpoints: [],
    children: [{
      name: 'sdb1', path: '/dev/sdb1', start_bytes: MIB, size_bytes: 499 * GIB,
      fstype: 'ntfs', mountpoints: [], holders: [],
    }],
    free_extents: [],
  };
  const defaults = chooseDefaults({ disks: [windows], live_medium: null });
  assert.equal(defaults.mode, MODE_MANUAL);
  assert.equal(defaults.extent, null);
});

check('an existing blank partition is selected only when it is blank', () => {
  const boot = {
    name: 'sdc', path: '/dev/sdc', size_bytes: 300 * GIB,
    serial: 'C1', pttype: 'gpt', removable: false, read_only: false,
    mountpoints: [],
    children: [
      {
        name: 'sdc1', path: '/dev/sdc1', start_bytes: MIB, size_bytes: 512 * MIB,
        fstype: 'vfat', partflags: ['esp'], mountpoints: [], holders: [],
      },
      {
        name: 'sdc2', path: '/dev/sdc2', start_bytes: 513 * MIB, size_bytes: 100 * GIB,
        fstype: '', mountpoints: [], holders: [],
      },
      {
        name: 'sdc3', path: '/dev/sdc3', start_bytes: 100 * GIB, size_bytes: 150 * GIB,
        fstype: 'ext4', mountpoints: [], holders: [],
      },
    ],
    free_extents: [],
  };
  const defaults = chooseDefaults({ disks: [boot], live_medium: null });
  assert.equal(defaults.mode, MODE_EXISTING);
  assert.equal(defaults.rootPath, '/dev/sdc2');
  assert.equal(defaults.espPath, '/dev/sdc1');
});

check('existing mode marks the chosen root and the reused ESP', () => {
  const state = {
    partitionMode: MODE_EXISTING,
    existingRootPath: '/dev/nvme0n1p2',
    layoutPreset: 'simple',
    swapGiB: 0,
    homeGiB: 0,
    manualSwapGiB: 0,
    manualHome: false,
    manualHomeGiB: 0,
    manualRootGiB: 0,
  };
  const model = buildPlanModel(disk, state);
  assert.equal(model.formatted.length, 1);
  assert.equal(model.formatted[0].path, '/dev/nvme0n1p2');
  assert.equal(model.kept.length, 1);
  assert.equal(model.after.filter((segment) => segment.selectedRoot).length, 1);
});

check('missing partition offsets are flagged as approximate', () => {
  const approx = {
    name: 'sdd', path: '/dev/sdd', size_bytes: 200 * GIB,
    serial: 'A1', pttype: 'gpt', removable: false, read_only: false,
    mountpoints: [],
    children: [{
      name: 'sdd1', path: '/dev/sdd1', start_bytes: 0, size_bytes: 100 * GIB,
      fstype: 'ext4', mountpoints: [], holders: [],
    }],
    free_extents: [],
  };
  const spans = diskSpans(approx);
  assert.equal(spans[0].approx, true);
  const gaps = diskGaps(approx);
  assert.equal(gaps.length, 1);
  assert.equal(gaps[0].source, 'approx');
  assert.equal(gaps[0].eligible, false);
});

check('mounted disks are refused in erase mode but not in manual mode', () => {
  const mounted = { ...disk, mountpoints: ['/mnt/data'] };
  assert.ok(diskReasons(mounted, MODE_ERASE).includes('mounted'));
  assert.ok(!diskReasons(mounted, MODE_MANUAL).includes('mounted'));
});

check('the summary reports formatted and kept partitions', () => {
  const summary = summaryFor(disk, manualState());
  assert.equal(summary.formattedPaths.length, 0);
  assert.equal(summary.keptCount, 2);
  assert.equal(summary.freeAfter, 0);
});

console.log(`\n${checks} timeline-model checks passed`);
