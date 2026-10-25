import {
  avatar, badge, button, controlButton, emptyState, field, icon, input,
  keyValue, select, spinner, toast,
} from '../../ui/index.js';
import { apiFetch } from '../../js/api.js';
import { t } from '../../js/i18n.js';
import {
  GIB, MIB,
  MODE_ERASE, MODE_EXISTING, MODE_MANUAL,
  applyPreset, buildPlanModel, chooseDefaults,
  diskGaps, diskReasons as baseDiskReasons, diskSpans, findEsp,
  isEsp, isSwap, layoutSpecsForPlan, placementForPlan,
  rootCandidates,
} from './timeline-model.js';
import { renderTimelinePanel } from './timeline.js';

const PLUGIN = 'installer';

const STEPS = [
  { id: 'region', icon: 'ui/keyboard' },
  { id: 'disk', icon: 'ui/monitor' },
  { id: 'account', icon: 'ui/user' },
  { id: 'profile', icon: 'ui/layers' },
  { id: 'review', icon: 'ui/list' },
];

const NVIDIA_DRIVER_LICENSE = {
  id: 'nvidia-driver-2025-02-25',
  version: '2025-02-25',
  url: '/plugins/installer/licenses/nvidia-driver.html',
};

const CUDA_LICENSE = {
  id: 'cuda-toolkit-eula',
  version: 'current',
  url: '/plugins/installer/licenses/cuda-eula.html',
};

const STAGE_KEYS = [
  ['prepare', 'progress.stage.prepare', 'Preparing'],
  ['partition', 'progress.stage.partition', 'Partitioning'],
  ['format', 'progress.stage.format', 'Formatting'],
  ['unpack', 'progress.stage.unpack', 'Unpacking rootfs'],
  ['configure', 'progress.stage.configure', 'Configuring'],
  ['bootloader', 'progress.stage.bootloader', 'Bootloader'],
  ['finish', 'progress.stage.finish', 'Finishing'],
];

const T = (key, fallback, params) => t(`plugin.installer.${key}`, params, fallback);

let tileEl = null;
let refs = {};
let pollTimer = null;
let state = null;
let fieldUid = 0;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function clear(node) {
  if (node) node.textContent = '';
}

function formatBytes(bytes) {
  const value = Number(bytes) || 0;
  if (value >= GIB) {
    const gib = value / GIB;
    return `${gib >= 10 ? gib.toFixed(1) : gib.toFixed(2)} GiB`;
  }
  if (value >= MIB) return `${(value / MIB).toFixed(0)} MiB`;
  return `${value} B`;
}

function defaultLocale() {
  const raw = navigator.language || 'en-US';
  const [language, region] = raw.split('-');
  const regions = {
    en: 'US', it: 'IT', de: 'DE', fr: 'FR', es: 'ES', pt: 'PT', nl: 'NL',
    pl: 'PL', ru: 'RU', uk: 'UA', sv: 'SE', fi: 'FI', da: 'DK', no: 'NO',
    cs: 'CZ', sk: 'SK', hu: 'HU', ro: 'RO', bg: 'BG', el: 'GR', tr: 'TR',
    ja: 'JP', ko: 'KR', zh: 'CN',
  };
  const territory = region || regions[language] || language.toUpperCase();
  return `${language}_${territory.toUpperCase()}.UTF-8`;
}

function defaultKeyboard() {
  const language = (navigator.language || 'en').split('-')[0].toLowerCase();
  const known = new Set([
    'it', 'us', 'gb', 'de', 'fr', 'es', 'pt', 'nl', 'pl', 'ru', 'uk',
    'sv', 'fi', 'da', 'no', 'cs', 'sk', 'hu', 'ro', 'bg', 'el', 'tr',
    'ja', 'ko', 'zh',
  ]);
  return known.has(language) ? language : 'us';
}

function defaultTimezone() {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch (_) {
    return 'UTC';
  }
}

function initialState() {
  return {
    step: 0,
    phase: 'wizard',
    loading: true,
    health: null,
    healthError: '',
    options: null,
    optionsError: '',
    optionsLoaded: false,
    probe: null,
    probeError: '',
    validation: null,
    validationKey: null,
    validationError: '',
    validating: false,
    showErrors: false,
    focusStep: false,
    locale: defaultLocale(),
    keyboard: defaultKeyboard(),
    timezone: defaultTimezone(),
    fullName: '',
    username: '',
    hostname: 'peakd',
    hostnameTouched: false,
    password: '',
    password2: '',
    profile: '',
    aiTier: 'voice',
    layoutPreset: 'swap',
    swapGiB: 8,
    homeGiB: 0,
    partitionMode: MODE_ERASE,
    selectedExtent: null,
    existingRootPath: '',
    existingEspPath: '',
    existingConfirmed: false,
    pendingExistingPath: '',
    manualSwapGiB: 8,
    manualHome: false,
    manualHomeGiB: 0,
    manualRootGiB: 0,
    timelineZoom: 1,
    exactOpen: false,
    gpuDriver: 'nouveau',
    installCuda: false,
    acceptedLicenses: {},
    mokAccepted: false,
    selectedDiskPath: '',
    confirmSerial: '',
    confirmed: false,
    job: null,
    jobStatus: null,
    jobError: '',
  };
}

function selectedDisk() {
  return (state.probe?.disks || []).find((disk) => disk.path === state.selectedDiskPath) || null;
}

function diskPaths(disk) {
  const paths = [];
  const walk = (node) => {
    if (node?.path) paths.push(node.path);
    for (const child of node?.children || []) walk(child);
  };
  walk(disk);
  return paths;
}

function diskHasMounts(disk) {
  return (disk?.mountpoints?.length || 0) > 0
    || (disk?.children || []).some(diskHasMounts);
}

function diskIsLive(disk) {
  const live = state.probe?.live_medium;
  return !!live && diskPaths(disk).includes(live);
}

function diskReasons(disk, mode = state.partitionMode) {
  return baseDiskReasons(disk, mode, state.probe?.live_medium);
}

function reasonLabel(reason) {
  const values = {
    missing: T('disk.reason.missing', 'Disk not found'),
    removable: T('disk.reason.removable', 'Removable device'),
    readOnly: T('disk.reason.readOnly', 'Read-only device'),
    loop: T('disk.reason.loop', 'Loop device'),
    optical: T('disk.reason.optical', 'Optical device'),
    mounted: T('disk.reason.mounted', 'A partition is mounted'),
    live: T('disk.reason.live', 'This is the live medium'),
    serial: T('disk.reason.serial', 'No serial exposed'),
    small: T('disk.reason.small', 'Smaller than 8 GiB'),
    table: T('disk.reason.table', 'Only GPT is supported'),
  };
  return values[reason] || reason;
}

function plannedBarLabel(kind) {
  if (kind === 'efi') return T('disk.efi', 'EFI system');
  if (kind === 'swap') return T('disk.swapSegment', 'Swap');
  if (kind === 'home') return T('disk.home', 'Home');
  return T('disk.root', "PEAK'D root");
}

function modelBarSegment(segment) {
  let label = segment.label;
  if (segment.planned) label = plannedBarLabel(segment.kind);
  else if (segment.kind === 'free') label = T('disk.free', 'Free space');
  else if (segment.selectedRoot) label = T('disk.root', "PEAK'D root");
  else if (segment.reused) label = T('disk.efi', 'EFI system');
  else if (segment.part) {
    label = segment.part.mountpoints?.[0] || segment.part.fstype || segment.part.name || segment.part.path;
  }
  return {
    kind: segment.kind,
    size: segment.size,
    label: label || segment.kind,
    detail: segment.part?.path || (segment.kind === 'free' ? T('disk.freeDetail', 'No partition table entry') : ''),
  };
}

function existingSegments(disk) {
  return diskSpans(disk).map(modelBarSegment);
}

function plannedSegments() {
  const disk = selectedDisk();
  if (!disk) return [];
  return buildPlanModel(disk, state).after.map(modelBarSegment);
}

function partitionRef(disk, part) {
  return {
    disk_path: disk?.path || '',
    disk_serial: disk?.serial || '',
    path: part?.path || '',
    partuuid: part?.partuuid || '',
    uuid: part?.uuid || '',
    size_bytes: part?.size_bytes || 0,
    fstype: part?.fstype || '',
  };
}

function preservedRefs(disk, exceptPath = '') {
  return (disk?.children || [])
    .filter((part) => part.path !== exceptPath)
    .map((part) => partitionRef(disk, part));
}

function selectedExistingRoot() {
  const disk = selectedDisk();
  if (!disk) return null;
  return (disk.children || []).find((part) => part.path === state.existingRootPath) || null;
}

function selectedExistingEsp() {
  const disk = selectedDisk();
  if (!disk) return null;
  return (disk.children || []).find((part) => part.path === state.existingEspPath) || null;
}

function layoutSpecs() {
  const disk = selectedDisk();
  if (!disk || state.partitionMode === MODE_EXISTING) return [];
  return layoutSpecsForPlan(disk, state);
}

function buildLayout(disk) {
  if (!disk) return { table: 'gpt', free_extent: null, partitions: [], preserve: [] };
  if (state.partitionMode === MODE_EXISTING) {
    return {
      table: 'gpt',
      free_extent: null,
      partitions: [],
      preserve: preservedRefs(disk, state.existingRootPath),
    };
  }
  return {
    table: 'gpt',
    free_extent: state.partitionMode === MODE_MANUAL ? placementForPlan(disk, state) : null,
    partitions: layoutSpecs(),
    preserve: state.partitionMode === MODE_MANUAL ? preservedRefs(disk) : [],
  };
}

function segmentBar(segments, { mini = false, label = '' } = {}) {
  const bar = el('div', `installer-bar${mini ? ' installer-bar--mini' : ''}`);
  bar.setAttribute('role', 'img');
  bar.setAttribute('aria-label', label || segments.map((segment) => `${segment.label}, ${formatBytes(segment.size)}`).join('; '));
  for (const segment of segments) {
    if (segment.size <= 0) continue;
    const part = el('span', `installer-bar-seg installer-bar-seg--${segment.kind}`);
    part.style.flex = `${segment.size} 1 0`;
    part.style.minWidth = '2px';
    part.title = `${segment.label} · ${formatBytes(segment.size)}${segment.detail ? ` · ${segment.detail}` : ''}`;
    bar.appendChild(part);
  }
  return bar;
}

function segmentLegend(segments) {
  const legend = el('div', 'installer-legend');
  for (const segment of segments) {
    if (segment.size <= 0) continue;
    const item = el('div', 'installer-legend-item');
    const dot = el('span', `installer-legend-dot installer-legend-dot--${segment.kind}`);
    const text = el('span', 'installer-legend-text');
    text.append(el('span', 'installer-legend-label', segment.label), el('span', 'installer-legend-value', formatBytes(segment.size)));
    item.append(dot, text);
    legend.appendChild(item);
  }
  return legend;
}

function note(kind, iconName, title, body) {
  const node = el('div', `installer-note${kind ? ` installer-note--${kind}` : ''}`);
  const mark = el('span', 'installer-note-icon');
  mark.appendChild(icon(iconName, { size: 18 }));
  const copy = el('div', 'installer-note-copy');
  if (title) copy.appendChild(el('div', 'installer-note-title', title));
  if (body) copy.appendChild(el('div', 'installer-note-body', body));
  node.append(mark, copy);
  return node;
}

function alertBlock(kind, title, body) {
  return note(kind, kind === 'danger' || kind === 'warn' ? 'ui/warning' : 'ui/info', title, body);
}

function sectionHeading(title, subtitle, actions = []) {
  const head = el('div', 'installer-section-head');
  const copy = el('div', 'installer-section-copy');
  copy.appendChild(el('h3', 'installer-section-title', title));
  if (subtitle) copy.appendChild(el('p', 'installer-section-sub', subtitle));
  head.appendChild(copy);
  if (actions.length) {
    const side = el('div', 'installer-section-actions');
    side.append(...actions.filter(Boolean));
    head.appendChild(side);
  }
  return head;
}

function fieldWithError({ label, hint, control, error, className = '' }) {
  const wrap = field({ label, hint, control });
  if (className) wrap.classList.add(className);
  if (error) {
    const target = control.select || control;
    target.setAttribute('aria-invalid', 'true');
    const id = `installer-error-${++fieldUid}`;
    target.setAttribute('aria-describedby', id);
    const message = el('p', 'installer-field-error', error);
    message.id = id;
    wrap.appendChild(message);
  }
  return wrap;
}

function createInput(key, options = {}) {
  const control = input({
    value: state[key] ?? '',
    placeholder: options.placeholder || '',
    type: options.type || 'text',
    autocomplete: options.autocomplete || '',
    onInput: (value) => {
      state[key] = value;
      if (key === 'hostname') state.hostnameTouched = true;
      if (key === 'username' && !state.hostnameTouched) {
        state.hostname = value ? `${value}-peakd` : 'peakd';
        if (refs.hostnameInput) refs.hostnameInput.value = state.hostname;
      }
      if (key === 'fullName' || key === 'username') updateAvatar();
      if (key === 'password' || key === 'password2') updatePasswordMeter();
      invalidateValidation();
      renderManifest();
      renderFooter();
    },
  });
  control.id = `installer-${key}`;
  return control;
}

function createSelect(key, values) {
  const current = state[key];
  const list = Array.isArray(values) ? values.filter(Boolean).slice() : [];
  if (current && !list.includes(current)) list.unshift(current);
  if (!list.length) list.push(current || '');
  const control = select({
    options: list.map((value) => ({ value, label: value })),
    value: current,
    onChange: (value) => {
      state[key] = value;
      invalidateValidation();
      renderManifest();
      renderFooter();
    },
  });
  control.select.id = `installer-${key}`;
  control.classList.add('installer-select');
  return control;
}

function updateAvatar() {
  if (!refs.avatar) return;
  refs.avatar.set({ name: state.fullName || state.username || '?' });
  if (refs.accountName) {
    refs.accountName.textContent = state.fullName || T('account.noName', 'Your name');
  }
  if (refs.accountLogin) {
    refs.accountLogin.textContent = state.username ? `@${state.username}` : T('account.noLogin', 'username');
  }
}

function updatePasswordMeter() {
  if (!refs.passwordReq) return;
  const checks = {
    length: state.password.length >= 8,
    quality: /\d|[^A-Za-z]/.test(state.password),
    match: state.password.length > 0 && state.password === state.password2,
  };
  for (const [key, node] of Object.entries(refs.passwordReq)) {
    node.classList.toggle('is-ok', !!checks[key]);
    const mark = node.querySelector('.installer-req-mark');
    if (mark) mark.textContent = checks[key] ? '✓' : '·';
  }
}

function invalidateValidation() {
  state.validation = null;
  state.validationKey = null;
  state.validationError = '';
}

function localFieldErrors() {
  const errors = {};
  const add = (key, message) => {
    if (!errors[key]) errors[key] = [];
    errors[key].push(message);
  };

  if (!state.locale.trim()) add('locale', T('region.locale.required', 'Locale is required'));
  if (!state.keyboard.trim()) add('keyboard', T('region.keyboard.required', 'Keyboard layout is required'));
  if (!state.timezone.trim()) add('timezone', T('region.timezone.required', 'Timezone is required'));

  if (!state.fullName.trim()) add('fullName', T('account.fullName.required', 'Full name is required'));
  if (!/^[a-z_][a-z0-9_-]{0,31}$/.test(state.username)) {
    add('username', T('account.username.invalid', 'Start with a lowercase letter or _; use lowercase letters, digits, _ or -'));
  }
  if (!/^[A-Za-z0-9](?:[A-Za-z0-9.-]{0,61}[A-Za-z0-9])?$/.test(state.hostname)) {
    add('hostname', T('account.hostname.invalid', 'Use 1–63 letters, digits, - or ., starting and ending with a letter or digit'));
  }
  if (state.password.length < 8) {
    add('password', T('account.password.short', 'Use at least 8 characters'));
  } else if (!/\d|[^A-Za-z]/.test(state.password)) {
    add('password', T('account.password.weak', 'Add a digit or a symbol'));
  }
  if (state.password !== state.password2) {
    add('password2', T('account.password.mismatch', 'The passwords do not match'));
  }

  if (!state.profile) add('profile', T('profile.required', 'Choose a kernel profile'));
  if (!['none', 'voice', 'local'].includes(state.aiTier)) add('aiTier', T('profile.ai.required', 'Choose an AI tier'));

  if (state.gpuDriver === 'nvidia') {
    if (!state.acceptedLicenses[NVIDIA_DRIVER_LICENSE.id]) {
      add('gpuLicense', T('profile.nvidia.license.required', 'Accept the NVIDIA driver license'));
    }
    if (state.installCuda && !state.acceptedLicenses[CUDA_LICENSE.id]) {
      add('cudaLicense', T('profile.nvidia.cudaLicense.required', 'Accept the CUDA EULA'));
    }
    if (state.health?.hardware?.secure_boot && !state.mokAccepted) {
      add('mok', T('profile.nvidia.mok.required', 'Acknowledge the MOK enrollment step'));
    }
  }

  return errors;
}

function modelErrorLabel(code) {
  const values = {
    noDisk: T('disk.noDisk', 'No disk selected'),
    noFree: T('disk.manual.noFree', 'No usable free space found on this disk'),
    noOffsets: T('disk.timeline.noOffsets', 'Partition positions are approximate; refresh the disks before changing the layout.'),
    gapTooSmall: T('disk.timeline.gapTooSmall', 'The free space is too small for PEAK’D!.'),
    swapTooBig: T('disk.timeline.swapTooBig', 'The support memory does not fit in this free space.'),
    homeTooBig: T('disk.manual.homeTooBig', 'Leave at least 20 GiB for the PEAK’D! system'),
    rootTooBig: T('disk.timeline.rootTooBig', 'The system does not fit in this free space.'),
    rootSmall: T('disk.manual.rootSmall', 'Leave at least 20 GiB for the system.'),
    noRoot: T('disk.existing.noRoot', 'Select an unmounted ext4 partition to use as root'),
    noEsp: T('disk.existing.noEsp', 'No EFI system partition found on this disk'),
  };
  return values[code] || code;
}

function diskLocalErrors() {
  const disk = selectedDisk();
  if (!disk) return [T('disk.noDisk', 'No disk selected')];
  const errors = diskReasons(disk).map(reasonLabel);
  const model = buildPlanModel(disk, state);
  errors.push(...model.errors.map(modelErrorLabel));

  if (state.partitionMode === MODE_EXISTING) {
    const root = selectedExistingRoot();
    const esp = selectedExistingEsp();
    if (!root) errors.push(T('disk.existing.noRoot', 'Select an unmounted ext4 partition to use as root'));
    if (!esp) errors.push(T('disk.existing.noEsp', 'No EFI system partition found on this disk'));
    if (root) {
      if ((root.mountpoints || []).length) {
        errors.push(T('disk.existing.rootMounted', 'The selected root partition is mounted'));
      }
      if ((root.holders || []).length) {
        errors.push(T('disk.existing.rootBusy', 'The selected root partition has active holders'));
      }
      if (root.fstype && root.fstype !== 'ext4') {
        errors.push(T('disk.existing.rootFilesystem', 'The root partition must be ext4 or empty'));
      }
      if (String(root.fstype || '').trim() && !state.existingConfirmed) {
        errors.push(T('disk.existing.needConfirm', 'Confirm that this partition will be formatted'));
      }
    }
    if (esp && esp.fstype !== 'vfat') {
      errors.push(T('disk.existing.espFilesystem', 'The startup partition must be vfat'));
    }
    if (root && esp && root.path === esp.path) {
      errors.push(T('disk.existing.samePartition', 'Root and startup partitions must differ'));
    }
  } else if (state.partitionMode === MODE_MANUAL) {
    if (state.manualHome && Number(state.manualHomeGiB) <= 0) {
      errors.push(T('disk.manual.homeSizeRequired', 'Enter the size for the personal-files partition'));
    }
  }

  return [...new Set(errors)];
}

function stepErrors(step) {
  const fields = localFieldErrors();
  if (step === 'region') {
    return ['locale', 'keyboard', 'timezone'].flatMap((key) => (fields[key] || []).map((message) => ({ key, message })));
  }
  if (step === 'disk') {
    return diskLocalErrors().map((message) => ({ key: 'disk', message }));
  }
  if (step === 'account') {
    return ['fullName', 'username', 'hostname', 'password', 'password2']
      .flatMap((key) => (fields[key] || []).map((message) => ({ key, message })));
  }
  if (step === 'profile') {
    return ['profile', 'aiTier', 'gpuLicense', 'cudaLicense', 'mok']
      .flatMap((key) => (fields[key] || []).map((message) => ({ key, message })));
  }
  return [];
}

function allLocalErrors() {
  return STEPS.flatMap((step) => stepErrors(step.id).map((error) => ({ ...error, step: step.id })));
}

function acceptedLicenseList() {
  return [NVIDIA_DRIVER_LICENSE, CUDA_LICENSE]
    .map((license) => state.acceptedLicenses[license.id])
    .filter(Boolean);
}

function buildNvidia() {
  const selected = state.gpuDriver === 'nvidia';
  const secureBoot = !!state.health?.hardware?.secure_boot;
  return {
    install_driver: selected,
    install_cuda: selected && state.installCuda,
    accept_driver_license: selected && !!state.acceptedLicenses[NVIDIA_DRIVER_LICENSE.id],
    accept_cuda_license: selected && state.installCuda && !!state.acceptedLicenses[CUDA_LICENSE.id],
    secure_boot_mok: selected && (!secureBoot || state.mokAccepted),
  };
}

function buildPlan() {
  const disk = selectedDisk();
  const root = state.partitionMode === MODE_EXISTING ? selectedExistingRoot() : null;
  const esp = state.partitionMode === MODE_EXISTING ? selectedExistingEsp() : null;
  return {
    target: disk
      ? {
        path: disk.path,
        serial: disk.serial || '',
        model: disk.model || '',
        size_bytes: disk.size_bytes || 0,
      }
      : { path: '', serial: '', model: '', size_bytes: 0 },
    target_partition: root && disk ? partitionRef(disk, root) : null,
    esp_partition: esp && disk ? partitionRef(disk, esp) : null,
    layout: buildLayout(disk),
    bootloader: 'grub',
    mode: state.partitionMode,
    filesystem: 'ext4',
    encryption: 'none',
    profile: state.profile || 'backports',
    locale: state.locale.trim(),
    keyboard: state.keyboard.trim(),
    timezone: state.timezone.trim(),
    full_name: state.fullName.trim(),
    username: state.username.trim(),
    password: state.password,
    hostname: state.hostname.trim(),
    install_voice: state.aiTier !== 'none',
    ai_tier: state.aiTier,
    nvidia: buildNvidia(),
    licenses: acceptedLicenseList(),
  };
}

function planKey() {
  return JSON.stringify(buildPlan());
}

function profileLabel(profile = state.profile) {
  return profile === 't2'
    ? T('profile.t2.title', 'T2 Mac')
    : T('profile.generic.title', 'Generic PC');
}

function aiLabel(tier = state.aiTier) {
  if (tier === 'none') return T('profile.ai.none.title', 'Text only');
  if (tier === 'local') return T('profile.ai.local.title', 'Local AI');
  return T('profile.ai.voice.title', 'Voice pack');
}

function layoutLabel(preset = state.layoutPreset) {
  if (preset === 'simple') return T('disk.preset.simple.title', 'Minimal');
  if (preset === 'home') return T('disk.preset.home.title', 'Separate home');
  return T('disk.preset.swap.title', 'Recommended');
}

function modeLabel(mode = state.partitionMode) {
  if (mode === MODE_EXISTING) return T('disk.mode.existing.title', 'Use a partition');
  if (mode === MODE_MANUAL) return T('disk.mode.manual.title', 'Manual layout');
  return T('disk.mode.erase.title', 'Erase disk');
}

function gpuLabel() {
  return state.gpuDriver === 'nvidia'
    ? T('profile.nvidia.proprietary.title', 'NVIDIA proprietary')
    : T('profile.nvidia.open.title', 'Open driver');
}

function acceptedLicenseRows() {
  return acceptedLicenseList().map((license) => [
    license.id,
    `${license.version} · ${license.accepted_at}`,
  ]);
}

function stageLabel(stage) {
  if (stage === 'queued') return T('progress.queued', 'Queued');
  if (stage === 'done') return T('progress.stage.done', 'Done');
  if (stage === 'failed') return T('progress.failed', 'Failed');
  const entry = STAGE_KEYS.find(([key]) => key === stage);
  return entry ? T(entry[1], entry[2]) : stage;
}

function stageIndex(stage) {
  return STAGE_KEYS.findIndex(([key]) => key === stage);
}

function statusPill(text, tone = 'neutral', { dot = false } = {}) {
  const node = el('span', `installer-status-pill installer-status-pill--${tone}`);
  if (dot) node.appendChild(el('span', 'installer-status-dot'));
  node.appendChild(el('span', null, text));
  return node;
}

function ensureStylesheet() {
  const id = 'installer-plugin-css';
  if (document.getElementById(id)) return;
  const link = document.createElement('link');
  link.id = id;
  link.rel = 'stylesheet';
  link.href = '/plugins/installer/plugin.css';
  document.head.appendChild(link);
}

async function refreshData() {
  state.loading = true;
  state.healthError = '';
  state.optionsError = '';
  state.probeError = '';
  render();
  try {
    const health = await apiFetch('/api/installer/health');
    if (!tileEl) return;
    state.health = health?.data ?? null;
    state.healthError = '';
    if (state.health && !state.profile) {
      state.profile = state.health.hardware?.is_t2 ? 't2' : 'backports';
    }
    if (state.health && !(state.health.hardware?.gpus || []).some((gpu) => gpu.is_nvidia)) {
      state.gpuDriver = 'nouveau';
      state.installCuda = false;
      state.mokAccepted = false;
    }
  } catch (error) {
    if (!tileEl) return;
    state.health = null;
    state.healthError = error.message;
  }

  if (state.health) {
    try {
      const response = await apiFetch('/api/installer/options');
      if (!tileEl) return;
      const options = response?.data ?? null;
      state.options = options;
      state.optionsError = '';
      if (options && !state.optionsLoaded) {
        state.locale = options.default_locale || state.locale;
        state.keyboard = options.default_keyboard || state.keyboard;
        state.timezone = options.default_timezone || state.timezone;
        state.optionsLoaded = true;
      }
    } catch (error) {
      if (!tileEl) return;
      state.options = null;
      state.optionsError = error.message;
    }

    try {
      const probe = await apiFetch('/api/installer/probe');
      if (!tileEl) return;
      state.probe = probe?.data ?? null;
      state.probeError = '';
      const disks = state.probe?.disks || [];
      const current = disks.find((disk) => disk.path === state.selectedDiskPath);
      if (!current) {
        applyDiskDefaults(chooseDefaults(state.probe));
      } else {
        state.selectedDiskPath = current.path;
        if (state.partitionMode === MODE_MANUAL) {
          const gaps = diskGaps(current)
            .filter((gap) => gap.eligible)
            .sort((a, b) => b.usableBytes - a.usableBytes);
          const wanted = state.selectedExtent
            ? gaps.find((gap) => gap.start_bytes === state.selectedExtent.start_bytes)
            : null;
          const gap = wanted || gaps[0] || null;
          const changed = !wanted;
          state.selectedExtent = gap
            ? { start_bytes: gap.start_bytes, size_bytes: gap.size_bytes }
            : null;
          if (gap && changed) {
            applyPreset(state, MODE_MANUAL, state.layoutPreset || 'swap', gap.size_bytes);
            state.confirmSerial = '';
            state.confirmed = false;
          }
        } else if (state.partitionMode === MODE_EXISTING) {
          autoSelectExisting(current);
        }
      }
    } catch (error) {
      if (!tileEl) return;
      state.probe = null;
      state.probeError = error.message;
    }
  }

  state.loading = false;
  render();
}

function applyDiskDefaults(defaults) {
  const disk = (state.probe?.disks || []).find((item) => item.path === defaults.diskPath) || null;
  state.selectedDiskPath = defaults.diskPath || '';
  state.partitionMode = defaults.mode || MODE_MANUAL;
  state.selectedExtent = defaults.extent
    ? { start_bytes: defaults.extent.start_bytes, size_bytes: defaults.extent.size_bytes }
    : null;
  state.existingRootPath = defaults.rootPath || '';
  state.existingEspPath = defaults.espPath || '';
  state.existingConfirmed = false;
  state.pendingExistingPath = '';
  const root = (disk?.children || []).find((part) => part.path === state.existingRootPath);
  if (root) {
    state.existingConfirmed = !String(root.fstype || '').trim();
    state.pendingExistingPath = state.existingConfirmed ? '' : root.path;
  }
  if (state.partitionMode === MODE_MANUAL && disk) {
    const gap = buildPlanModel(disk, state).gap;
    applyPreset(state, MODE_MANUAL, defaults.preset || state.layoutPreset || 'swap', gap?.size_bytes || 0);
  } else if (state.partitionMode === MODE_ERASE && disk) {
    applyPreset(state, MODE_ERASE, defaults.preset || state.layoutPreset || 'swap', disk.size_bytes);
  }
}

function render() {
  if (!tileEl || !state) return;
  renderChrome();
  renderSteps();
  renderMain();
  renderFooter();
  renderManifest();
}

function renderChrome() {
  const chrome = refs.chrome;
  clear(chrome);
  const brand = el('div', 'installer-brand');
  brand.appendChild(el('span', 'installer-mark app-icon'));
  const brandCopy = el('div', 'installer-brand-copy');
  brandCopy.append(
    el('span', 'installer-brand-name', "PEAK'D!"),
    el('span', 'installer-brand-sub', T('chrome.installer', 'Installer')),
  );
  brand.appendChild(brandCopy);

  const status = el('div', 'installer-chrome-status');
  if (state.health) {
    status.appendChild(statusPill(T('chrome.live', 'Live session'), 'ok', { dot: true }));
    if (state.health.dry_run) {
      status.appendChild(statusPill(T('chrome.dryRun', 'Dry run'), 'accent'));
    }
    const totalSteps = state.phase === 'progress' ? STEPS.length + 1 : STEPS.length;
    const currentStep = state.phase === 'progress' ? totalSteps : Math.min(state.step + 1, STEPS.length);
    status.appendChild(statusPill(T('chrome.step', 'Step {current} of {total}', {
      current: currentStep,
      total: totalSteps,
    }), 'neutral'));
  } else if (state.loading) {
    status.appendChild(statusPill(T('chrome.connecting', 'Checking helper'), 'neutral'));
  } else {
    status.appendChild(statusPill(T('chrome.offline', 'Helper offline'), 'error', { dot: true }));
  }
  chrome.append(brand, el('span', 'installer-chrome-spacer'), status);
}

function renderSteps() {
  const nav = refs.steps;
  clear(nav);
  const head = el('div', 'installer-steps-head');
  head.append(
    el('span', 'installer-eyebrow', T('steps.eyebrow', 'Setup')),
    el('h2', 'installer-steps-title', T('steps.title', 'Installation')),
  );
  nav.appendChild(head);

  const list = el('div', 'installer-step-list');
  STEPS.forEach((step, index) => {
    const active = state.phase === 'wizard' && state.step === index;
    const done = state.phase === 'progress' || index < state.step;
    const item = controlButton();
    item.type = 'button';
    item.className = `installer-step-item${active ? ' is-active' : ''}${done ? ' is-done' : ''}`;
    item.disabled = state.phase === 'progress';
    if (active) item.setAttribute('aria-current', 'step');
    const label = T(`steps.${step.id}.label`, step.id);
    const desc = T(`steps.${step.id}.desc`, '');
    item.title = label;
    item.setAttribute('aria-label', `${label} — ${desc}`);
    item.addEventListener('click', () => goStep(index));
    const dot = el('span', 'installer-step-dot');
    dot.appendChild(icon(done ? 'ui/check' : step.icon, { size: 15 }));
    const copy = el('span', 'installer-step-copy');
    copy.append(
      el('span', 'installer-step-kicker', `0${index + 1}`),
      el('span', 'installer-step-label', label),
      el('span', 'installer-step-desc', desc),
    );
    item.append(dot, copy);
    list.appendChild(item);
  });

  if (state.phase === 'progress') {
    const item = el('div', 'installer-step-item is-active is-installing');
    item.setAttribute('aria-current', 'step');
    const dot = el('span', 'installer-step-dot');
    dot.appendChild(spinner({ size: 14 }));
    const copy = el('span', 'installer-step-copy');
    copy.append(
      el('span', 'installer-step-kicker', '06'),
      el('span', 'installer-step-label', T('steps.progress.label', 'Installing')),
      el('span', 'installer-step-desc', T('steps.progress.desc', 'Live progress')),
    );
    item.append(dot, copy);
    list.appendChild(item);
  }
  nav.appendChild(list);
  requestAnimationFrame(() => {
    const active = list.querySelector('.installer-step-item.is-active');
    active?.scrollIntoView({ block: 'nearest', inline: 'center' });
  });
}

function renderMain(options = {}) {
  if (!refs.inner) return;
  const scrollTop = refs.scroll.scrollTop;
  clear(refs.inner);
  if (state.phase === 'progress') {
    renderProgress();
  } else if (state.loading) {
    renderLoading();
  } else if (!state.health) {
    renderHelperError();
  } else {
    renderStep();
  }
  refs.scroll.scrollTop = options.resetScroll ? 0 : scrollTop;
  if (state.focusStep) {
    const heading = refs.inner.querySelector('.installer-step-title');
    if (heading) {
      heading.tabIndex = -1;
      heading.focus({ preventScroll: true });
    }
    state.focusStep = false;
  }
}

function renderLoading() {
  const wrap = el('div', 'installer-loading');
  wrap.append(spinner({ size: 22 }), el('p', 'installer-loading-text', T('state.loading', 'Reading the machine and checking the installer helper…')));
  refs.inner.appendChild(wrap);
}

function renderHelperError() {
  const wrap = el('div', 'installer-state');
  wrap.appendChild(emptyState({
    icon: 'ui/warning',
    title: T('state.helperDown.title', 'The installer helper is not running'),
    body: state.healthError || T('state.helperDown.body', 'Start shiny-installer-helper in the live session, then retry.'),
    action: button({
      label: T('common.retry', 'Retry'),
      variant: 'primary',
      icon: 'ui/refresh',
      onClick: () => void refreshData(),
    }),
  }));
  refs.inner.appendChild(wrap);
}

function stepHead(step) {
  const index = STEPS.indexOf(step);
  const head = el('header', 'installer-step-head');
  const mark = el('div', 'installer-step-mark');
  mark.appendChild(icon(step.icon, { size: 22 }));
  const copy = el('div', 'installer-step-head-copy');
  const kicker = el('div', 'installer-step-kicker', index >= 0 ? `0${index + 1} / 05` : 'INSTALL');
  const title = el('h2', 'installer-step-title', T(`steps.${step.id}.title`, step.id));
  title.tabIndex = -1;
  const subtitle = el('p', 'installer-step-subtitle', T(`steps.${step.id}.subtitle`, ''));
  copy.append(kicker, title, subtitle);
  head.append(mark, copy);
  return head;
}

function renderStep() {
  const step = STEPS[state.step] || STEPS[0];
  clear(refs.inner);
  const wrap = el('div', 'installer-step');
  wrap.appendChild(stepHead(step));
  if (step.id === 'region') wrap.appendChild(renderRegion());
  if (step.id === 'disk') wrap.appendChild(renderDisk());
  if (step.id === 'account') wrap.appendChild(renderAccount());
  if (step.id === 'profile') wrap.appendChild(renderProfile());
  if (step.id === 'review') wrap.appendChild(renderReview());
  refs.inner.appendChild(wrap);
  if (step.id === 'review') {
    queueMicrotask(() => void maybeValidatePlan());
  }
}

function renderRegion() {
  const errors = localFieldErrors();
  const wrap = el('div', 'installer-stack');
  const grid = el('div', 'installer-grid installer-grid--2');
  const options = state.options;
  const hasOptions = !!options
    && Array.isArray(options.locales) && options.locales.length > 0
    && Array.isArray(options.keyboards) && options.keyboards.length > 0
    && Array.isArray(options.timezones) && options.timezones.length > 0;
  const locale = hasOptions ? createSelect('locale', options.locales) : createInput('locale');
  const keyboard = hasOptions ? createSelect('keyboard', options.keyboards) : createInput('keyboard');
  const timezone = hasOptions ? createSelect('timezone', options.timezones) : createInput('timezone');
  grid.append(
    fieldWithError({
      label: T('region.locale.label', 'Locale'),
      hint: T('region.locale.hint', 'Locale installed on this system.'),
      control: locale,
      error: state.showErrors ? errors.locale?.[0] : '',
    }),
    fieldWithError({
      label: T('region.keyboard.label', 'Keyboard layout'),
      hint: T('region.keyboard.hint', 'Keyboard layouts reported by the system.'),
      control: keyboard,
      error: state.showErrors ? errors.keyboard?.[0] : '',
    }),
    fieldWithError({
      label: T('region.timezone.label', 'Timezone'),
      hint: T('region.timezone.hint', 'Timezones reported by the system.'),
      control: timezone,
      error: state.showErrors ? errors.timezone?.[0] : '',
    }),
  );
  wrap.appendChild(grid);
  if (!hasOptions && state.optionsError) {
    wrap.appendChild(alertBlock(
      'warn',
      T('region.options.errorTitle', 'System lists unavailable'),
      T('region.options.errorBody', 'Could not read the system option lists. Type the values instead.'),
    ));
  }
  wrap.appendChild(note(
    'accent',
    'ui/info',
    T('region.live.title', 'The live session stays yours'),
    T('region.live.body', "Keep using PEAK'D while the installer prepares the target. Nothing is written until you press Install."),
  ));
  const detected = el('div', 'installer-detected');
  detected.append(
    el('span', 'installer-eyebrow', T('region.detected', 'Detected from this session')),
    badge(`${state.options?.default_locale || navigator.language || 'en-US'} · ${state.options?.default_timezone || defaultTimezone()}`, { tone: 'neutral' }),
  );
  wrap.appendChild(detected);
  return wrap;
}

function renderDisk() {
  const wrap = el('div', 'installer-stack');
  if (state.probeError) {
    wrap.appendChild(alertBlock(
      'danger',
      T('disk.probeError.title', 'Could not read the disks'),
      state.probeError,
    ));
    wrap.appendChild(button({
      label: T('common.retry', 'Retry'),
      variant: 'ghost',
      icon: 'ui/refresh',
      onClick: () => void refreshData(),
    }));
    return wrap;
  }

  const disks = state.probe?.disks || [];
  if (!disks.length) {
    wrap.appendChild(emptyState({
      icon: 'ui/monitor',
      title: T('disk.empty.title', 'No installable disks'),
      body: T('disk.empty.body', 'The live medium, loop devices and optical drives are hidden. Plug in a target disk or refresh.'),
      action: button({
        label: T('disk.refresh', 'Refresh disks'),
        variant: 'ghost',
        icon: 'ui/refresh',
        onClick: () => void refreshData(),
      }),
    }));
    return wrap;
  }

  wrap.appendChild(renderDiskShelf(disks));
  const disk = selectedDisk();
  if (!disk) return wrap;

  const model = buildPlanModel(disk, state);
  const reasons = diskReasons(disk);
  if (reasons.length) {
    wrap.appendChild(alertBlock(
      'danger',
      T('disk.ineligible.title', 'This disk cannot be used'),
      reasons.map(reasonLabel).join(' · '),
    ));
  }

  wrap.appendChild(renderIntentSelector(disk));
  wrap.appendChild(renderTimelinePanel({
    disk,
    state,
    model,
    t: T,
    formatBytes,
    onSelectGap: selectGap,
    onSelectPartition: selectExistingPartition,
    onPreset: setLayoutPreset,
    onRemovePartition: removePlannedPartition,
    onCommit: (nextState, nextModel) => {
      state = nextState;
      invalidateValidation();
      state.showErrors = false;
      renderFooter();
      renderManifest();
      updateTimelineSummaryState(nextModel);
    },
    onRequestRender: () => {
      invalidateValidation();
      state.showErrors = false;
      renderMain();
      renderFooter();
      renderManifest();
    },
  }));

  if (state.partitionMode === MODE_EXISTING && state.pendingExistingPath && !state.existingConfirmed) {
    wrap.appendChild(renderExistingConfirm(disk));
  }
  wrap.appendChild(renderGuidedNotes(disk, model));
  wrap.appendChild(renderPartitionListFallback(disk));
  return wrap;
}

function updateTimelineSummaryState() {
  if (refs.live) {
    refs.live.textContent = T('disk.timeline.updated', 'Disk layout updated');
  }
}

function renderDiskShelf(disks) {
  const wrap = el('section', 'installer-disks');
  wrap.appendChild(sectionHeading(
    T('disk.device', 'Install target'),
    T('disk.deviceHint', 'One disk. The helper re-checks the exact model, serial and size before it touches anything.'),
  ));
  const shelf = el('div', 'installer-diskshelf');
  shelf.setAttribute('role', 'radiogroup');
  shelf.setAttribute('aria-label', T('disk.device', 'Install target'));
  const ordered = [...disks].sort((a, b) => {
    const aBad = diskReasons(a).length ? 1 : 0;
    const bBad = diskReasons(b).length ? 1 : 0;
    return aBad - bBad || Number(b.size_bytes) - Number(a.size_bytes);
  });
  for (const disk of ordered) shelf.appendChild(diskCard(disk));
  wrap.appendChild(shelf);
  return wrap;
}

function renderIntentSelector(disk) {
  const wrap = el('section', 'installer-intent-wrap');
  wrap.appendChild(sectionHeading(
    T('disk.intent.title', 'What should happen to the files?'),
    T('disk.intent.hint', 'The timeline shows exactly where PEAK’D! will live.'),
  ));
  const choices = el('div', 'installer-intent-row');
  choices.setAttribute('role', 'radiogroup');
  choices.setAttribute('aria-label', T('disk.intent.title', 'What should happen to the files?'));

  const gaps = diskGaps(disk).filter((gap) => gap.eligible);
  const roots = rootCandidates(disk);
  const esp = findEsp(disk);
  const options = [
    {
      mode: MODE_MANUAL,
      iconName: 'ui/layers',
      title: T('disk.intent.free', 'Use the free space'),
      desc: gaps.length
        ? T('disk.intent.free.desc', 'Install next to the existing partitions. Nothing already on the disk is resized or moved.')
        : T('disk.intent.free.none', 'There is no usable free space on this disk.'),
      badge: gaps.length ? T('disk.intent.recommended', 'Recommended') : '',
      disabled: !gaps.length,
    },
    {
      mode: MODE_ERASE,
      iconName: 'ui/trash',
      title: T('disk.intent.erase', 'Erase the whole disk'),
      desc: T('disk.intent.erase.desc', 'Start from zero. Every partition on this disk will be deleted.'),
      disabled: diskReasons(disk, MODE_ERASE).length > 0,
    },
    {
      mode: MODE_EXISTING,
      iconName: 'ui/monitor',
      title: T('disk.intent.partition', 'Use one partition'),
      desc: roots.length && esp
        ? T('disk.intent.partition.desc', 'Format one existing partition and keep the others.')
        : T('disk.intent.partition.none', 'This disk has no suitable partition and startup partition.'),
      disabled: !roots.length || !esp,
    },
  ];

  for (const option of options) {
    const card = el('button', `installer-intent${state.partitionMode === option.mode ? ' is-selected' : ''}`);
    card.type = 'button';
    card.disabled = option.disabled;
    card.setAttribute('role', 'radio');
    card.setAttribute('aria-checked', String(state.partitionMode === option.mode));
    card.addEventListener('click', () => setPartitionMode(option.mode));
    const mark = el('span', 'installer-intent-icon');
    mark.appendChild(icon(option.iconName, { size: 18 }));
    const copy = el('span', 'installer-intent-copy');
    copy.append(
      el('span', 'installer-intent-title', option.title),
      el('span', 'installer-intent-desc', option.desc),
    );
    if (option.badge) copy.appendChild(badge(option.badge, { tone: 'accent' }));
    card.append(mark, copy);
    choices.appendChild(card);
  }
  wrap.appendChild(choices);
  return wrap;
}

function setPartitionMode(mode) {
  if (state.partitionMode === mode) return;
  const disk = selectedDisk();
  state.partitionMode = mode;
  state.existingRootPath = '';
  state.existingEspPath = '';
  state.existingConfirmed = false;
  state.pendingExistingPath = '';
  state.selectedExtent = null;
  state.confirmSerial = '';
  state.confirmed = false;
  state.showErrors = false;
  if (disk && mode === MODE_MANUAL) {
    const gap = diskGaps(disk)
      .filter((item) => item.eligible)
      .sort((a, b) => b.usableBytes - a.usableBytes)[0];
    if (gap) {
      state.selectedExtent = { start_bytes: gap.start_bytes, size_bytes: gap.size_bytes };
      applyPreset(state, MODE_MANUAL, state.layoutPreset || 'swap', gap.size_bytes);
    }
  }
  if (disk && mode === MODE_EXISTING) autoSelectExisting(disk);
  if (disk && mode === MODE_ERASE) {
    applyPreset(state, MODE_ERASE, state.layoutPreset || 'swap', disk.size_bytes);
  }
  invalidateValidation();
  renderMain();
  renderFooter();
  renderManifest();
}

function autoSelectExisting(disk, { preferBlank = true } = {}) {
  const roots = rootCandidates(disk);
  let root = roots.find((part) => part.path === state.existingRootPath) || null;
  if (!root && preferBlank) {
    root = roots.find((part) => !String(part.fstype || '').trim()) || null;
  }
  state.existingRootPath = root?.path || '';
  state.existingEspPath = findEsp(disk)?.path || '';
  state.existingConfirmed = !!root && !String(root.fstype || '').trim();
  state.pendingExistingPath = root && String(root.fstype || '').trim() ? root.path : '';
}

function selectGap(gap) {
  if (!gap) return;
  const disk = selectedDisk();
  state.partitionMode = MODE_MANUAL;
  state.selectedExtent = { start_bytes: gap.start_bytes, size_bytes: gap.size_bytes };
  state.existingRootPath = '';
  state.existingEspPath = '';
  state.existingConfirmed = false;
  state.pendingExistingPath = '';
  state.confirmSerial = '';
  state.confirmed = false;
  state.showErrors = false;
  applyPreset(state, MODE_MANUAL, state.layoutPreset || 'swap', gap.size_bytes);
  invalidateValidation();
  if (disk) renderMain();
  renderFooter();
  renderManifest();
}

function selectExistingPartition(part) {
  if (!part) return;
  const disk = selectedDisk();
  state.partitionMode = MODE_EXISTING;
  state.existingRootPath = part.path;
  state.existingEspPath = findEsp(disk)?.path || '';
  state.existingConfirmed = !String(part.fstype || '').trim();
  state.pendingExistingPath = state.existingConfirmed ? '' : part.path;
  state.selectedExtent = null;
  state.confirmSerial = '';
  state.confirmed = false;
  state.showErrors = false;
  invalidateValidation();
  renderMain();
  renderFooter();
  renderManifest();
}

function confirmExisting() {
  if (!state.pendingExistingPath) return;
  state.existingConfirmed = true;
  state.pendingExistingPath = '';
  state.showErrors = false;
  invalidateValidation();
  renderMain();
  renderFooter();
  renderManifest();
}

function cancelExisting() {
  state.existingRootPath = '';
  state.pendingExistingPath = '';
  state.existingConfirmed = false;
  state.showErrors = false;
  invalidateValidation();
  renderMain();
  renderFooter();
  renderManifest();
}

function renderExistingConfirm(disk) {
  const part = (disk.children || []).find((item) => item.path === state.pendingExistingPath);
  const wrap = el('section', 'installer-existing-confirm');
  wrap.appendChild(alertBlock(
    'warn',
    T('disk.existing.confirmTitle', 'This partition will be formatted'),
    T('disk.existing.confirmBody', 'Everything stored in {path} will be deleted. The other partitions are not touched.', {
      path: part?.path || state.pendingExistingPath,
    }),
  ));
  const actions = el('div', 'installer-existing-actions');
  actions.append(
    button({
      label: T('common.cancel', 'Cancel'),
      variant: 'ghost',
      onClick: cancelExisting,
    }),
    button({
      label: T('disk.existing.confirm', 'Format this partition and install here'),
      variant: 'danger',
      icon: 'ui/trash',
      onClick: confirmExisting,
    }),
  );
  wrap.appendChild(actions);
  return wrap;
}

function removePlannedPartition(kind) {
  if (state.partitionMode === MODE_MANUAL) {
    if (kind === 'swap') state.manualSwapGiB = 0;
    if (kind === 'home') {
      state.manualHome = false;
      state.manualHomeGiB = 0;
    }
  } else {
    if (kind === 'swap') state.swapGiB = 0;
    if (kind === 'home') {
      state.homeGiB = 0;
      state.layoutPreset = state.swapGiB > 0 ? 'swap' : 'simple';
    }
  }
  state.confirmSerial = '';
  state.confirmed = false;
  state.showErrors = false;
  invalidateValidation();
  renderMain();
  renderFooter();
  renderManifest();
}

function renderGuidedNotes(disk, model) {
  const wrap = el('div', 'installer-guided-notes');
  const roots = rootCandidates(disk);
  const gaps = diskGaps(disk).filter((gap) => gap.eligible);
  if (!gaps.length && !roots.length && (disk.children || []).length) {
    wrap.appendChild(note(
      'warn',
      'ui/warning',
      T('disk.guided.noSpaceTitle', 'No room for PEAK’D!'),
      T('disk.guided.noSpaceBody', 'This disk has no usable free space and no partition PEAK’D! can reuse. Erase the disk or free some space from the other system, then refresh.'),
    ));
  }
  if ((disk.children || []).some((part) => {
    const fstype = String(part.fstype || '').toLowerCase();
    const typename = String(part.parttypename || '').toLowerCase();
    return fstype === 'apfs' || fstype === 'hfs' || typename.includes('apfs') || typename.includes('hfs');
  })) {
    wrap.appendChild(note(
      'accent',
      'ui/info',
      T('disk.guided.apfsTitle', 'Apple partitions stay untouched'),
      T('disk.guided.apfsBody', 'APFS and HFS partitions are preserved. PEAK’D! can only use the free space or a partition you choose.'),
    ));
  }
  if (model.approx) {
    wrap.appendChild(note(
      'warn',
      'ui/warning',
      T('disk.timeline.approxTitle', 'Approximate disk map'),
      T('disk.timeline.noOffsets', 'Partition positions are approximate; refresh the disks before changing the layout.'),
    ));
  }
  if (!findEsp(disk) && state.partitionMode === MODE_EXISTING) {
    wrap.appendChild(alertBlock(
      'danger',
      T('disk.existing.noEspTitle', 'Startup partition missing'),
      T('disk.existing.noEspBody', 'This disk has no FAT32 startup partition. PEAK’D! needs one to boot; use “Use the free space” or erase the disk.'),
    ));
  }
  return wrap;
}

function renderPartitionListFallback(disk) {
  const roots = rootCandidates(disk);
  const details = el('details', 'installer-list-fallback');
  details.appendChild(el('summary', 'installer-list-fallback-summary', T('disk.view.list', 'Partition list')));
  if (!roots.length) {
    details.appendChild(el('p', 'installer-hint', T('disk.existing.noRootBody', 'This disk has no unused or formattable partition with at least 20 GiB.')));
    return details;
  }
  const list = el('div', 'installer-partition-list');
  list.setAttribute('role', 'radiogroup');
  list.setAttribute('aria-label', T('disk.existing.pickTitle', 'Where should PEAK’D! live?'));
  for (const part of roots) list.appendChild(partitionCard(disk, part));
  details.appendChild(list);
  return details;
}

function partitionCard(disk, part) {
  const active = state.existingRootPath === part.path;
  const card = controlButton();
  card.type = 'button';
  card.className = `installer-partition${active ? ' is-selected' : ''}`;
  card.setAttribute('role', 'radio');
  card.setAttribute('aria-checked', String(active));
  card.setAttribute('aria-label', `${T('disk.existing.pickOne', 'Install PEAK’D! in this partition')}: ${part.name || part.path}`);
  card.addEventListener('click', () => selectExistingPartition(part));
  const mark = el('span', 'installer-partition-icon');
  mark.appendChild(icon('ui/layers', { size: 17 }));
  const main = el('div', 'installer-partition-main');
  const title = part.mountpoints?.[0] || `${T('disk.partition.label', 'Partition')} ${part.name || part.path.split('/').pop()}`;
  main.append(
    el('span', 'installer-partition-title', title),
    el('span', 'installer-partition-meta', `${formatBytes(part.size_bytes)} · ${part.fstype || T('disk.partition.blank', 'not formatted')}`),
    el('span', 'installer-partition-detail', active
      ? T('disk.existing.selected', 'PEAK’D! will be installed here. This partition will be formatted.')
      : T('disk.existing.other', 'Your files stay here. This partition will not be touched.')),
    el('span', 'installer-partition-path', part.path),
  );
  const badges = el('div', 'installer-partition-badges');
  if (active) badges.appendChild(badge(T('disk.existing.selectedBadge', 'Selected'), { tone: 'accent' }));
  if (isEsp(part)) badges.appendChild(badge(T('disk.partition.esp', 'Startup'), { tone: 'neutral' }));
  if (isSwap(part)) badges.appendChild(badge(T('disk.partition.swap', 'Swap'), { tone: 'warn' }));
  if ((part.mountpoints || []).length) badges.appendChild(badge(T('disk.badge.mounted', 'In use'), { tone: 'warn' }));
  if ((part.holders || []).length) badges.appendChild(badge(T('disk.partition.busy', 'Busy'), { tone: 'error' }));
  if (badges.children.length) main.appendChild(badges);
  const check = el('span', 'installer-partition-check');
  if (active) check.appendChild(icon('ui/check', { size: 12 }));
  card.append(mark, main, check);
  return card;
}

function diskCard(disk) {
  const active = disk.path === state.selectedDiskPath;
  const reasons = diskReasons(disk);
  const card = controlButton();
  card.type = 'button';
  card.className = `installer-disk${active ? ' is-selected' : ''}${reasons.length ? ' has-reasons' : ''}`;
  card.setAttribute('role', 'radio');
  card.setAttribute('aria-checked', String(active));
  card.addEventListener('click', () => selectDisk(disk.path));

  const mark = el('span', 'installer-disk-icon');
  mark.appendChild(icon('ui/monitor', { size: 20 }));

  const main = el('div', 'installer-disk-main');
  const title = el('div', 'installer-disk-title');
  title.append(
    el('span', 'installer-disk-name', disk.path),
    el('span', 'installer-disk-model', disk.model || T('disk.unknownModel', 'Disk')),
  );
  main.appendChild(title);
  const freeGap = diskGaps(disk)
    .filter((gap) => gap.eligible)
    .sort((a, b) => b.size_bytes - a.size_bytes)[0];
  const meta = [
    disk.tran ? disk.tran.toUpperCase() : '',
    `${(disk.children || []).length} ${T('disk.partitionCount', 'partitions')}`,
    freeGap
      ? `${T('disk.free', 'Free space')} ${formatBytes(freeGap.size_bytes)}`
      : T('disk.noFreeShort', 'No free space'),
    disk.serial ? `${T('common.serial', 'Serial')} ${disk.serial}` : '',
  ].filter(Boolean).join(' · ');
  if (reasons.length) card.title = reasons.map(reasonLabel).join(' · ');
  main.appendChild(el('div', 'installer-disk-meta', meta));
  const badges = el('div', 'installer-disk-badges');
  if (diskHasMounts(disk)) badges.appendChild(badge(T('disk.badge.mounted', 'Mounted'), { tone: 'warn' }));
  if (disk.removable) badges.appendChild(badge(T('disk.badge.removable', 'Removable'), { tone: 'warn' }));
  if (disk.read_only) badges.appendChild(badge(T('disk.badge.readOnly', 'Read-only'), { tone: 'warn' }));
  if (diskIsLive(disk)) badges.appendChild(badge(T('disk.badge.live', 'Live medium'), { tone: 'error' }));
  if (!reasons.length) badges.appendChild(badge(T('disk.badge.eligible', 'Eligible'), { tone: 'ok' }));
  else badges.appendChild(badge(T('disk.badge.notEligible', 'Not eligible'), { tone: 'error' }));
  main.appendChild(badges);
  main.appendChild(segmentBar(existingSegments(disk), { mini: true, label: T('disk.currentBar', 'Current partition layout for {path}', { path: disk.path }) }));

  const side = el('div', 'installer-disk-side');
  side.append(
    el('span', 'installer-disk-size', formatBytes(disk.size_bytes)),
    el('span', 'installer-disk-serial', disk.serial || T('disk.noSerial', 'No serial exposed')),
  );
  const check = el('span', 'installer-disk-check');
  if (active) check.appendChild(icon('ui/check', { size: 14 }));
  card.append(mark, main, side, check);
  return card;
}

function selectDisk(path) {
  if (state.selectedDiskPath === path) return;
  state.selectedDiskPath = path;
  state.confirmSerial = '';
  state.confirmed = false;
  state.existingRootPath = '';
  state.existingEspPath = '';
  state.existingConfirmed = false;
  state.pendingExistingPath = '';
  state.selectedExtent = null;
  const disk = selectedDisk();
  if (disk) {
    const gaps = diskGaps(disk)
      .filter((gap) => gap.eligible)
      .sort((a, b) => b.usableBytes - a.usableBytes);
    if (gaps.length) {
      state.partitionMode = MODE_MANUAL;
      state.selectedExtent = {
        start_bytes: gaps[0].start_bytes,
        size_bytes: gaps[0].size_bytes,
      };
      applyPreset(state, MODE_MANUAL, state.layoutPreset || 'swap', gaps[0].size_bytes);
    } else if (!(disk.children || []).length) {
      state.partitionMode = MODE_ERASE;
      applyPreset(state, MODE_ERASE, state.layoutPreset || 'swap', disk.size_bytes);
    } else if (rootCandidates(disk).length && findEsp(disk)) {
      state.partitionMode = MODE_EXISTING;
      autoSelectExisting(disk);
    } else {
      state.partitionMode = MODE_MANUAL;
    }
  }
  invalidateValidation();
  state.showErrors = false;
  renderMain();
  renderFooter();
  renderManifest();
}

function setLayoutPreset(value) {
  const disk = selectedDisk();
  if (!disk) return;
  const model = buildPlanModel(disk, state);
  const rawBytes = state.partitionMode === MODE_MANUAL
    ? (model.gap?.size_bytes || 0)
    : disk.size_bytes;
  applyPreset(state, state.partitionMode, value, rawBytes);
  state.confirmSerial = '';
  state.confirmed = false;
  state.showErrors = false;
  invalidateValidation();
  renderMain();
  renderFooter();
  renderManifest();
}

function choiceCard({ value, current, iconName, title, desc, meta, disabled = false, onClick }) {
  const card = controlButton();
  card.type = 'button';
  card.className = `installer-choice${current ? ' is-selected' : ''}${disabled ? ' is-disabled' : ''}`;
  card.dataset.value = value;
  card.setAttribute('role', 'radio');
  card.setAttribute('aria-checked', String(current));
  card.disabled = disabled;
  if (onClick) card.addEventListener('click', onClick);
  const mark = el('span', 'installer-choice-icon');
  mark.appendChild(icon(iconName, { size: 18 }));
  const copy = el('span', 'installer-choice-copy');
  copy.append(
    el('span', 'installer-choice-title', title),
    el('span', 'installer-choice-desc', desc),
  );
  if (meta) copy.appendChild(el('span', 'installer-choice-meta', meta));
  const check = el('span', 'installer-choice-check');
  if (current) check.appendChild(icon('ui/check', { size: 12 }));
  card.append(mark, copy, check);
  return card;
}

function renderAccount() {
  const errors = localFieldErrors();
  const wrap = el('div', 'installer-account');
  const card = el('div', 'installer-account-card');
  refs.avatar = avatar({ name: state.fullName || state.username || '?', size: 'lg' });
  refs.avatar.classList.add('installer-avatar-big');
  refs.accountName = el('div', 'installer-account-name', state.fullName || T('account.noName', 'Your name'));
  refs.accountLogin = el('div', 'installer-account-login', state.username ? `@${state.username}` : T('account.noLogin', 'username'));
  card.append(
    refs.avatar,
    refs.accountName,
    refs.accountLogin,
    badge(T('account.owner', "PEAK'D owner"), { tone: 'accent' }),
    badge(T('account.admin', 'Administrator'), { tone: 'neutral' }),
  );
  wrap.appendChild(card);

  const fields = el('div', 'installer-account-fields');
  const fullName = createInput('fullName', { placeholder: 'Ada Lovelace', autocomplete: 'name' });
  const username = createInput('username', { placeholder: 'ada', autocomplete: 'username' });
  const hostname = createInput('hostname', { placeholder: 'peakd', autocomplete: 'off' });
  refs.hostnameInput = hostname;
  fields.appendChild(fieldWithError({
    label: T('account.fullName.label', 'Full name'),
    hint: T('account.fullName.hint', "The name shown on the lock screen and in the PEAK'D profile."),
    control: fullName,
    error: state.showErrors ? errors.fullName?.[0] : '',
  }));
  const pair = el('div', 'installer-grid installer-grid--2');
  pair.append(
    fieldWithError({
      label: T('account.username.label', 'Username'),
      hint: T('account.username.hint', 'Lowercase letters, digits, _ or -; starts with a letter or _.'),
      control: username,
      error: state.showErrors ? errors.username?.[0] : '',
    }),
    fieldWithError({
      label: T('account.hostname.label', 'Hostname'),
      hint: T('account.hostname.hint', 'The name other machines see on the network.'),
      control: hostname,
      error: state.showErrors ? errors.hostname?.[0] : '',
    }),
  );
  fields.appendChild(pair);

  const passwords = el('div', 'installer-grid installer-grid--2');
  passwords.append(
    fieldWithError({
      label: T('account.password.label', 'Password'),
      control: createInput('password', { type: 'password', autocomplete: 'new-password' }),
      error: state.showErrors ? errors.password?.[0] : '',
    }),
    fieldWithError({
      label: T('account.password2.label', 'Confirm password'),
      control: createInput('password2', { type: 'password', autocomplete: 'new-password' }),
      error: state.showErrors ? errors.password2?.[0] : '',
    }),
  );
  fields.appendChild(passwords);

  const req = el('div', 'installer-password-req');
  refs.passwordReq = {};
  const reqs = [
    ['length', T('account.req.length', '8+ characters')],
    ['quality', T('account.req.quality', 'A digit or symbol')],
    ['match', T('account.req.match', 'Passwords match')],
  ];
  for (const [key, label] of reqs) {
    const item = el('span', 'installer-req');
    item.append(el('span', 'installer-req-mark', '·'), el('span', null, label));
    refs.passwordReq[key] = item;
    req.appendChild(item);
  }
  fields.appendChild(req);
  fields.appendChild(note(
    'warn',
    'ui/lock',
    T('account.password.dryTitle', 'Your account password'),
    T('account.password.dryBody', 'The installer will set this password on your new account. It is sent only to the privileged helper and is not written to the plan or logs.'),
  ));
  updatePasswordMeter();
  wrap.appendChild(fields);
  return wrap;
}

function renderProfile() {
  const hw = state.health?.hardware || {};
  const wrap = el('div', 'installer-stack');
  const hardware = el('div', 'installer-hardware');
  const mark = el('span', 'installer-hardware-icon');
  mark.appendChild(icon('ui/monitor', { size: 20 }));
  const copy = el('div', 'installer-hardware-copy');
  copy.append(
    el('span', 'installer-hardware-name', hw.product || T('profile.unknownHardware', 'Unknown hardware')),
    el('span', 'installer-hardware-meta', [hw.vendor, hw.board].filter(Boolean).join(' · ') || T('profile.hardwareMeta', 'DMI information unavailable')),
  );
  const badges = el('div', 'installer-hardware-badges');
  badges.appendChild(badge(hw.is_t2 ? T('profile.detectedT2', 'T2 bridge detected') : T('profile.detectedGeneric', 'Generic x86_64'), {
    tone: hw.is_t2 ? 'accent' : 'neutral',
  }));
  hardware.append(mark, copy, badges);
  wrap.appendChild(hardware);

  wrap.appendChild(sectionHeading(
    T('profile.kernel.title', 'Kernel profile'),
    T('profile.kernel.hint', 'Two kernels ship in the live image. This choice decides what the installed system boots.'),
  ));
  const profiles = el('div', 'installer-layout-choices');
  profiles.setAttribute('role', 'radiogroup');
  profiles.setAttribute('aria-label', T('profile.kernel.title', 'Kernel profile'));
  profiles.append(
    choiceCard({
      value: 'backports',
      current: state.profile === 'backports',
      iconName: 'ui/monitor',
      title: T('profile.generic.title', 'Generic PC'),
      desc: T('profile.generic.desc', 'Pinned backports kernel, Mesa and PipeWire for x86_64.'),
      meta: T('profile.generic.meta', 'linux-image-amd64 · backports'),
      onClick: () => {
        state.profile = 'backports';
        invalidateValidation();
        renderMain();
        renderFooter();
        renderManifest();
      },
    }),
    choiceCard({
      value: 't2',
      current: state.profile === 't2',
      iconName: 'ui/shield',
      title: T('profile.t2.title', 'T2 Mac'),
      desc: T('profile.t2.desc', 'linux-t2 and its companions for the Apple T2 bridge.'),
      meta: hw.is_t2 ? T('profile.t2.meta', 'linux-t2 · detected') : T('profile.t2.missing', 'T2 bridge not detected'),
      disabled: !hw.is_t2,
      onClick: () => {
        state.profile = 't2';
        invalidateValidation();
        renderMain();
        renderFooter();
        renderManifest();
      },
    }),
  );
  wrap.appendChild(profiles);
  if (hw.is_t2 && state.profile !== 't2') {
    wrap.appendChild(alertBlock(
      'warn',
      T('profile.generic.warnTitle', 'Generic profile on T2 hardware'),
      T('profile.generic.warnBody', 'Keyboard, trackpad, Wi-Fi, Bluetooth or audio may not be enabled. Choose the T2 profile for this machine.'),
    ));
  }
  wrap.appendChild(note(
    'accent',
    'ui/layers',
    T('profile.kernel.noteTitle', 'One live image, two profiles'),
    T('profile.kernel.noteBody', 'The generic profile installs the pinned backports kernel. The T2 profile installs linux-t2 and does not install the generic kernel.'),
  ));

  wrap.appendChild(sectionHeading(
    T('profile.ai.title', 'AI tier'),
    T('profile.ai.hint', 'Choose how much of the local AI stack to provision. You can add models later.'),
  ));
  const tiers = el('div', 'installer-layout-choices installer-layout-choices--ai');
  tiers.setAttribute('role', 'radiogroup');
  tiers.setAttribute('aria-label', T('profile.ai.title', 'AI tier'));
  const aiOptions = [
    ['none', 'ui/keyboard', T('profile.ai.none.title', 'Text only'), T('profile.ai.none.desc', 'Chat and agent, no voice models.'), T('profile.ai.none.meta', 'Smallest footprint')],
    ['voice', 'ui/download', T('profile.ai.voice.title', 'Voice pack'), T('profile.ai.voice.desc', 'Speech recognition and neural voices.'), T('profile.ai.voice.meta', 'Recommended')],
    ['local', 'ui/layers', T('profile.ai.local.title', 'Local AI'), T('profile.ai.local.desc', 'Voice plus local image and music models.'), T('profile.ai.local.meta', 'Needs more disk')],
  ];
  for (const [value, iconName, title, desc, meta] of aiOptions) {
    tiers.appendChild(choiceCard({
      value,
      current: state.aiTier === value,
      iconName,
      title,
      desc,
      meta,
      onClick: () => {
        state.aiTier = value;
        invalidateValidation();
        renderMain();
        renderFooter();
        renderManifest();
      },
    }));
  }
  wrap.appendChild(tiers);
  wrap.appendChild(renderNvidiaSection());
  return wrap;
}

function renderNvidiaSection() {
  const wrap = el('div', 'installer-stack');
  const gpus = state.health?.hardware?.gpus || [];
  const nvidia = gpus.find((gpu) => gpu.is_nvidia);
  wrap.appendChild(sectionHeading(
    T('profile.nvidia.title', 'NVIDIA driver & CUDA'),
    T('profile.nvidia.hint', 'Optional proprietary driver and CUDA runtime. The open driver is the default.'),
  ));
  if (!nvidia) {
    wrap.appendChild(note(
      'accent',
      'ui/info',
      T('profile.nvidia.noGpuTitle', 'No NVIDIA GPU detected'),
      T('profile.nvidia.noGpuBody', 'This machine keeps the open driver and software rendering. No NVIDIA license is needed.'),
    ));
    return wrap;
  }

  const hardware = el('div', 'installer-hardware');
  const mark = el('span', 'installer-hardware-icon');
  mark.appendChild(icon('ui/monitor', { size: 20 }));
  const copy = el('div', 'installer-hardware-copy');
  copy.append(
    el('span', 'installer-hardware-name', nvidia.model || 'NVIDIA'),
    el('span', 'installer-hardware-meta', [
      nvidia.pci_id,
      nvidia.is_hybrid ? T('profile.nvidia.hybrid', 'Hybrid graphics') : '',
      state.health?.hardware?.secure_boot ? T('profile.nvidia.secureBoot', 'Secure Boot enabled') : T('profile.nvidia.noSecureBoot', 'Secure Boot off'),
    ].filter(Boolean).join(' · ')),
  );
  const badges = el('div', 'installer-hardware-badges');
  badges.appendChild(badge(T('profile.nvidia.detected', 'NVIDIA detected'), { tone: 'accent' }));
  if (nvidia.is_hybrid) badges.appendChild(badge(T('profile.nvidia.hybrid', 'Hybrid graphics'), { tone: 'neutral' }));
  hardware.append(mark, copy, badges);
  wrap.appendChild(hardware);

  const choices = el('div', 'installer-layout-choices installer-gpu-choices');
  choices.setAttribute('role', 'radiogroup');
  choices.setAttribute('aria-label', T('profile.nvidia.title', 'NVIDIA driver & CUDA'));
  choices.append(
    choiceCard({
      value: 'nouveau',
      current: state.gpuDriver === 'nouveau',
      iconName: 'ui/monitor',
      title: T('profile.nvidia.open.title', 'Open driver'),
      desc: T('profile.nvidia.open.desc', 'In-tree nouveau, no third-party license, lower performance.'),
      onClick: () => {
        state.gpuDriver = 'nouveau';
        state.installCuda = false;
        invalidateValidation();
        renderMain();
        renderFooter();
        renderManifest();
      },
    }),
    choiceCard({
      value: 'nvidia',
      current: state.gpuDriver === 'nvidia',
      iconName: 'ui/shield',
      title: T('profile.nvidia.proprietary.title', 'NVIDIA proprietary'),
      desc: T('profile.nvidia.proprietary.desc', 'NVIDIA driver for full performance, with optional CUDA.'),
      meta: T('profile.nvidia.proprietary.meta', 'Requires license acceptance'),
      disabled: !!nvidia.is_datacenter,
      onClick: () => {
        state.gpuDriver = 'nvidia';
        invalidateValidation();
        renderMain();
        renderFooter();
        renderManifest();
      },
    }),
  );
  wrap.appendChild(choices);

  if (nvidia.is_datacenter) {
    wrap.appendChild(alertBlock(
      'danger',
      T('profile.nvidia.datacenterTitle', 'Data-center GPU'),
      T('profile.nvidia.datacenterBody', 'The consumer NVIDIA driver path is not offered for this GPU.'),
    ));
  }

  if (state.gpuDriver === 'nvidia') {
    wrap.appendChild(note(
      'warn',
      'ui/warning',
      T('profile.nvidia.warnTitle', 'Proprietary driver'),
      T('profile.nvidia.warnBody', 'nouveau is unloaded and the kernel is tainted. You can switch back after install.'),
    ));
    wrap.appendChild(licenseRow(NVIDIA_DRIVER_LICENSE, T('profile.nvidia.license.driver', 'NVIDIA Driver License')));
    const cudaRow = el('label', 'installer-confirm-check installer-consent');
    const cudaCheck = input({ type: 'checkbox' });
    cudaCheck.checked = state.installCuda;
    cudaCheck.addEventListener('change', () => {
      state.installCuda = cudaCheck.checked;
      invalidateValidation();
      renderMain();
      renderFooter();
      renderManifest();
    });
    cudaRow.append(cudaCheck, el('span', 'installer-confirm-check-text', T('profile.nvidia.cuda', 'Also install CUDA (optional)')));
    wrap.appendChild(cudaRow);
    if (state.installCuda) {
      wrap.appendChild(licenseRow(CUDA_LICENSE, T('profile.nvidia.license.cuda', 'CUDA Toolkit EULA')));
    }
    if (state.health?.hardware?.secure_boot) {
      const mokRow = el('label', 'installer-confirm-check installer-consent');
      const mokCheck = input({ type: 'checkbox' });
      mokCheck.checked = state.mokAccepted;
      mokCheck.addEventListener('change', () => {
        state.mokAccepted = mokCheck.checked;
        invalidateValidation();
        renderFooter();
        renderManifest();
      });
      mokRow.append(mokCheck, el('span', 'installer-confirm-check-text', T('profile.nvidia.mok', 'I understand the MOK enrollment step will be required at first boot')));
      wrap.appendChild(mokRow);
      wrap.appendChild(el('p', 'installer-hint', T('profile.nvidia.mokHint', 'Secure Boot needs the NVIDIA DKMS module signed through MOK.')));
    }
    if (nvidia.is_hybrid) {
      wrap.appendChild(note(
        'accent',
        'ui/info',
        T('profile.nvidia.hybridTitle', 'Hybrid graphics'),
        T('profile.nvidia.hybridBody', 'The integrated GPU stays enabled. PEAK\'D! uses PRIME/offload for the NVIDIA GPU.'),
      ));
    }
  }
  return wrap;
}

function licenseRow(license, label) {
  const row = el('div', 'installer-consent');
  const choice = el('label', 'installer-confirm-check');
  const box = input({ type: 'checkbox' });
  box.checked = !!state.acceptedLicenses[license.id];
  box.addEventListener('change', () => {
    if (box.checked) {
      state.acceptedLicenses = {
        ...state.acceptedLicenses,
        [license.id]: {
          id: license.id,
          version: license.version,
          hash: '',
          accepted_at: new Date().toISOString(),
        },
      };
    } else {
      const next = { ...state.acceptedLicenses };
      delete next[license.id];
      state.acceptedLicenses = next;
    }
    invalidateValidation();
    renderFooter();
    renderManifest();
  });
  choice.append(box, el('span', 'installer-confirm-check-text', T('profile.nvidia.license.accept', 'I have read and accept')));
  const link = el('a', 'installer-license-link', label);
  link.href = license.url;
  link.target = '_blank';
  link.rel = 'noopener noreferrer';
  row.append(choice, link, el('span', 'installer-hint', `(${license.version})`));
  return row;
}

function reviewBlock(title, body) {
  const block = el('section', 'installer-review-block');
  block.append(el('h3', 'installer-review-block-title', title), body);
  return block;
}

function renderReview() {
  const disk = selectedDisk();
  const errors = localFieldErrors();
  const wrap = el('div', 'installer-review');
  if (state.health?.dry_run) {
    wrap.appendChild(note(
      'warn',
      'ui/warning',
      T('review.dryTitle', 'Simulation mode'),
      T('review.dryBody', 'Pressing Install simulates the plan. No disk is modified in this build.'),
    ));
  } else {
    wrap.appendChild(note(
      'danger',
      'ui/warning',
      T('review.realTitle', 'This will change the disk'),
      T('review.realBody', 'The partitions shown below will be formatted. Check the disk, the serial and the layout before continuing.'),
    ));
  }

  const reviewGrid = el('div', 'installer-review-grid');
  reviewGrid.appendChild(reviewBlock(T('review.region', 'Language & region'), keyValue([
    [T('region.locale.label', 'Locale'), state.locale],
    [T('region.keyboard.label', 'Keyboard layout'), state.keyboard],
    [T('region.timezone.label', 'Timezone'), state.timezone],
  ])));

  const diskBody = el('div', 'installer-review-disk');
  const model = disk ? buildPlanModel(disk, state) : null;
  if (disk && model) {
    const root = selectedExistingRoot();
    const esp = selectedExistingEsp();
    const rows = [
      [T('review.mode', 'Mode'), modeLabel()],
      [T('common.device', 'Device'), disk.path],
      [T('common.model', 'Model'), disk.model || '—'],
      [T('common.serial', 'Serial'), disk.serial || '—'],
      [T('common.size', 'Size'), formatBytes(disk.size_bytes)],
    ];
    if (state.partitionMode === MODE_EXISTING) {
      rows.push([T('review.rootPartition', 'Root partition'), root?.path || '—']);
      rows.push([T('review.espPartition', 'EFI system partition'), esp?.path || '—']);
    } else if (model.gap) {
      rows.push([T('disk.timeline.zone', 'Install zone'), `${formatBytes(model.gap.start_bytes)} – ${formatBytes(model.gap.start_bytes + model.gap.size_bytes)}`]);
    }
    if (state.partitionMode !== MODE_ERASE) {
      rows.push([T('review.preserved', 'Preserved partitions'), String(model.kept.length)]);
    }
    rows.push([T('disk.summary.formatted', 'Will be formatted'), model.formatted.length
      ? model.formatted.map((part) => part.path).join(', ')
      : T('disk.summary.nothingTouched', 'No existing partition will be touched')]);
    diskBody.appendChild(keyValue(rows));
    diskBody.appendChild(segmentBar(existingSegments(disk), { mini: true, label: T('disk.currentBar', 'Current partition layout for {path}', { path: disk.path }) }));
    diskBody.appendChild(segmentBar(plannedSegments(), { label: T('disk.afterBar', 'Planned partition layout') }));
  } else {
    diskBody.appendChild(el('p', 'installer-review-empty', T('review.noDisk', 'No disk selected.')));
  }
  reviewGrid.appendChild(reviewBlock(T('review.destination', 'Destination'), diskBody));

  reviewGrid.appendChild(reviewBlock(T('review.account', 'Account'), keyValue([
    [T('account.fullName.label', 'Full name'), state.fullName || '—'],
    [T('account.username.label', 'Username'), state.username || '—'],
    [T('account.hostname.label', 'Hostname'), state.hostname || '—'],
    [T('account.password.label', 'Password'), T('review.passwordValue', 'Set during install')],
  ])));

  const systemRows = [
    [T('profile.kernel.title', 'Kernel profile'), profileLabel()],
    [T('profile.ai.title', 'AI tier'), aiLabel()],
    [T('review.mode', 'Mode'), modeLabel()],
    [T('review.gpu', 'GPU driver'), gpuLabel()],
    [T('review.cuda', 'CUDA'), state.gpuDriver === 'nvidia' && state.installCuda ? T('common.yes', 'Yes') : T('common.no', 'No')],
  ];
  if (state.health?.hardware?.secure_boot && state.gpuDriver === 'nvidia') {
    systemRows.push([T('review.mok', 'Secure Boot MOK'), state.mokAccepted ? T('common.acknowledged', 'Acknowledged') : T('common.required', 'Required')]);
  }
  reviewGrid.appendChild(reviewBlock(T('review.system', 'System'), keyValue(systemRows)));

  if (acceptedLicenseRows().length) {
    reviewGrid.appendChild(reviewBlock(
      T('review.licenses', 'Accepted licenses'),
      keyValue(acceptedLicenseRows()),
    ));
  }
  wrap.appendChild(reviewGrid);

  if (state.validating) {
    const validating = el('div', 'installer-validation is-loading');
    validating.append(spinner({ size: 16 }), el('span', null, T('review.validating', 'Checking target with the helper…')));
    wrap.appendChild(validating);
  } else if (state.validationError) {
    wrap.appendChild(alertBlock('danger', T('review.validationFailed', 'Validation failed'), state.validationError));
  } else if (state.validation) {
    const errorsList = state.validation.errors || [];
    const warnings = state.validation.warnings || [];
    if (!errorsList.length && !warnings.length) {
      wrap.appendChild(note(
        'ok',
        'ui/check',
        T('review.valid', 'Target validated'),
        T('review.validBody', 'The helper matched the disk identity and the plan is ready for the engine.'),
      ));
    }
    if (errorsList.length) {
      wrap.appendChild(alertBlock('danger', T('review.errors', 'Target needs attention'), errorsList.join(' · ')));
    }
    if (warnings.length) {
      wrap.appendChild(alertBlock('warn', T('review.warnings', 'Warnings'), warnings.join(' · ')));
    }
  }

  const localMessages = Object.values(errors).flat();
  if (localMessages.length) {
    const local = el('div', 'installer-inline-errors');
    for (const message of localMessages) local.appendChild(el('p', 'installer-inline-error', message));
    wrap.appendChild(local);
  }

  const confirm = el('section', 'installer-confirm');
  const confirmHead = el('div', 'installer-confirm-head');
  const confirmMark = el('span', 'installer-confirm-icon');
  confirmMark.appendChild(icon('ui/warning', { size: 20 }));
  const confirmCopy = el('div', 'installer-confirm-copy');
  confirmCopy.append(
    el('h3', 'installer-confirm-title', T('review.confirm.title', 'Confirm the target')),
    el('p', 'installer-confirm-body', T('review.confirm.body', 'Type the disk serial exactly, then acknowledge the operation.')),
  );
  confirmHead.append(confirmMark, confirmCopy);
  confirm.appendChild(confirmHead);

  const serial = input({
    value: state.confirmSerial,
    placeholder: disk?.serial || '...',
    autocomplete: 'off',
    onInput: (value) => {
      state.confirmSerial = value;
      renderFooter();
    },
  });
  serial.id = 'installer-confirm-serial';
  confirm.appendChild(field({
    label: T('review.confirm.serial', 'Disk serial'),
    hint: disk?.serial ? `${T('review.confirm.expected', 'Expected')}: ${disk.serial}` : T('review.confirm.noSerial', 'This disk exposes no serial; install is unavailable.'),
    control: serial,
  }));
  if (disk?.serial && state.confirmSerial && state.confirmSerial.trim() !== disk.serial) {
    confirm.appendChild(el('p', 'installer-field-error', T('review.serialMismatch', 'Type the exact disk serial')));
  }

  const checkLabel = el('label', 'installer-confirm-check');
  const check = input({ type: 'checkbox' });
  check.checked = state.confirmed;
  check.addEventListener('change', () => {
    state.confirmed = check.checked;
    if (check.checked && state.partitionMode === MODE_EXISTING && state.pendingExistingPath) {
      state.existingConfirmed = true;
      state.pendingExistingPath = '';
      invalidateValidation();
      renderMain();
    }
    renderFooter();
  });
  const confirmKey = state.partitionMode === MODE_EXISTING
    ? 'review.confirm.checkExisting'
    : state.partitionMode === MODE_MANUAL
      ? 'review.confirm.checkManual'
      : 'review.confirm.checkErase';
  const confirmFallback = state.partitionMode === MODE_EXISTING
    ? 'I understand the selected root partition will be formatted; every other partition will be preserved.'
    : state.partitionMode === MODE_MANUAL
      ? 'I understand the planned partitions will be created and formatted in free space; every existing partition will be preserved.'
      : 'I understand every partition on this disk will be erased.';
  checkLabel.append(check, el('span', 'installer-confirm-check-text', T(confirmKey, confirmFallback)));
  confirm.appendChild(checkLabel);

  wrap.appendChild(confirm);
  if (state.showErrors && localMessages.length) {
    queueMicrotask(focusFirstInvalid);
  }
  return wrap;
}

async function maybeValidatePlan() {
  if (!tileEl || state.step !== 4 || state.phase !== 'wizard') return;
  if (state.validating || !state.health || state.validationError) return;
  if (allLocalErrors().length || !selectedDisk()) return;
  if (state.validationKey === planKey()) return;
  await validatePlan();
}

async function validatePlan() {
  if (!tileEl || state.validating) return;
  if (allLocalErrors().length || !selectedDisk()) return;
  const key = planKey();
  state.validating = true;
  state.validationError = '';
  if (state.step === 4) {
    renderMain();
    renderFooter();
  }
  try {
    const response = await apiFetch('/api/installer/plan', {
      method: 'POST',
      body: JSON.stringify(buildPlan()),
    });
    if (!tileEl) return;
    state.validation = response?.data ?? null;
    state.validationKey = key;
  } catch (error) {
    if (!tileEl) return;
    state.validation = null;
    state.validationKey = null;
    state.validationError = error.message;
  }
  state.validating = false;
  if (state.step === 4) {
    renderMain();
    renderFooter();
    renderManifest();
  }
}

function installReady() {
  const disk = selectedDisk();
  if (!disk || !state.health || state.phase !== 'wizard' || state.step !== 4) return false;
  if (allLocalErrors().length) return false;
  if (diskLocalErrors().length) return false;
  if (!state.validationKey || state.validationKey !== planKey()) return false;
  if (!state.validation?.ok) return false;
  if (!state.confirmed) return false;
  return state.confirmSerial.trim() === disk.serial;
}

function renderFooter() {
  const footer = refs.footer;
  clear(footer);
  const step = el('div', 'installer-footer-step');
  const notReady = state.phase === 'wizard' && state.step === STEPS.length - 1 && !installReady();
  if (state.phase === 'progress') {
    step.textContent = state.jobStatus?.stage
      ? `${stageLabel(state.jobStatus.stage)} · ${state.jobStatus.progress || 0}%`
      : T('progress.queued', 'Queued');
  } else if (notReady) {
    step.textContent = `${T('common.step', 'Step {current} of {total}', {
      current: Math.min(state.step + 1, STEPS.length),
      total: STEPS.length,
    })} · ${T('review.notReadyShort', 'Serial and confirmation required')}`;
  } else {
    step.textContent = T('common.step', 'Step {current} of {total}', {
      current: Math.min(state.step + 1, STEPS.length),
      total: STEPS.length,
    });
  }
  footer.appendChild(step);
  if (refs.live) refs.live.textContent = step.textContent;

  const actions = el('div', 'installer-footer-actions');
  if (state.phase === 'progress') {
    if (state.jobStatus?.done) {
      actions.appendChild(button({
        label: T('progress.back', 'Back to review'),
        variant: 'primary',
        icon: 'ui/check',
        onClick: backToReview,
      }));
    } else {
      actions.appendChild(button({
        label: T('progress.cancel', 'Cancel install'),
        variant: 'danger',
        onClick: () => void cancelJob(),
      }));
    }
  } else {
    actions.appendChild(button({
      label: T('common.back', 'Back'),
      variant: 'ghost',
      disabled: state.step === 0,
      onClick: () => goStep(state.step - 1),
    }));
    if (state.step < STEPS.length - 1) {
      actions.appendChild(button({
        label: T('common.next', 'Next'),
        variant: 'primary',
        onClick: goNext,
      }));
    } else {
      if (state.validationKey !== planKey() || state.validationError) {
        actions.appendChild(button({
          label: T('review.recheck', 'Re-check plan'),
          variant: 'ghost',
          icon: 'ui/refresh',
          onClick: () => void validatePlan(),
        }));
      }
      const install = button({
        label: T('common.install', "Install PEAK'D!"),
        variant: 'primary',
        icon: 'ui/download',
        disabled: !installReady(),
        onClick: () => void startInstall(),
      });
      if (!installReady()) {
        install.title = T('review.notReady', 'Complete the fields and confirm the disk serial');
      }
      actions.appendChild(install);
    }
  }
  footer.appendChild(actions);
}

function goStep(index) {
  if (state.phase === 'progress' && !state.jobStatus?.done) return;
  const next = Math.max(0, Math.min(STEPS.length - 1, index));
  state.phase = 'wizard';
  state.step = next;
  state.showErrors = false;
  state.focusStep = true;
  render();
  refs.scroll.scrollTop = 0;
}

function goNext() {
  const errors = stepErrors(STEPS[state.step]?.id);
  if (errors.length) {
    state.showErrors = true;
    renderMain();
    queueMicrotask(focusFirstInvalid);
    return;
  }
  if (state.step === STEPS.length - 1) {
    void startInstall();
    return;
  }
  goStep(state.step + 1);
}

function focusFirstInvalid() {
  const invalid = refs.inner?.querySelector('[aria-invalid="true"]');
  if (invalid) {
    invalid.focus();
    return;
  }
  const error = refs.inner?.querySelector('.installer-inline-error');
  if (error) error.scrollIntoView({ block: 'center', behavior: 'smooth' });
}

async function startInstall() {
  const local = allLocalErrors();
  if (local.length) {
    state.showErrors = true;
    const first = local[0];
    const index = STEPS.findIndex((step) => step.id === first.step);
    if (index >= 0) goStep(index);
    toast(T('review.fixFirst', 'Fix the highlighted fields first'), { type: 'error' });
    return;
  }
  const disk = selectedDisk();
  if (!disk) {
    toast(T('review.noDisk', 'No disk selected'), { type: 'error' });
    return;
  }
  if (!state.confirmed) {
    toast(T('review.needConfirm', 'Acknowledge the erase first'), { type: 'error' });
    return;
  }
  if (state.confirmSerial.trim() !== disk.serial) {
    toast(T('review.serialMismatch', 'Type the exact disk serial'), { type: 'error' });
    return;
  }
  if (!state.validation?.ok || state.validationKey !== planKey()) {
    await validatePlan();
    if (!state.validation?.ok) {
      toast(T('review.targetNotValid', 'The helper has not validated this target'), { type: 'error' });
      return;
    }
  }

  state.phase = 'progress';
  state.job = null;
  state.jobError = '';
  state.jobStatus = {
    job: '',
    stage: 'queued',
    progress: 0,
    done: false,
    success: false,
    error: null,
    log: [T('progress.accepted', 'Install request accepted')],
  };
  render();
  try {
    const response = await apiFetch('/api/installer/apply', {
      method: 'POST',
      body: JSON.stringify({
        plan: buildPlan(),
        confirm_serial: state.confirmSerial.trim(),
      }),
    });
    if (!tileEl) return;
    state.job = response?.data?.job || null;
    if (!state.job) throw new Error(T('progress.noJob', 'The helper did not return a job'));
    startPolling();
  } catch (error) {
    if (!tileEl) return;
    state.phase = 'wizard';
    state.jobError = error.message;
    render();
    toast(error.message, { type: 'error' });
  }
}

function startPolling() {
  if (pollTimer) clearInterval(pollTimer);
  const tick = async () => {
    if (!state.job) return;
    try {
      const response = await apiFetch(`/api/installer/status?job=${encodeURIComponent(state.job)}`);
      if (!tileEl) return;
      const status = response?.data;
      if (!status) return;
      state.jobStatus = status;
      const finished = status.done;
      if (state.phase === 'progress') {
        if (finished) {
          renderMain();
          renderFooter();
          renderManifest();
        } else {
          updateProgressDom();
        }
      }
      if (finished) {
        clearInterval(pollTimer);
        pollTimer = null;
        if (status.success) {
          toast(T('progress.complete', 'Installation complete'), { type: 'info' });
        } else {
          toast(status.error || T('progress.failed', 'Installation failed'), { type: 'error' });
        }
      }
    } catch (error) {
      clearInterval(pollTimer);
      pollTimer = null;
      if (!tileEl) return;
      state.jobError = error.message;
      if (state.phase === 'progress') {
        renderMain();
        renderFooter();
      }
      toast(error.message, { type: 'error' });
    }
  };
  void tick();
  pollTimer = window.setInterval(tick, 700);
}

function paintStage(node, key, status, current) {
  const index = stageIndex(key);
  const failed = status.done && !status.success && key === status.stage;
  const done = status.done ? status.success : index < current;
  const active = !status.done && key === status.stage;
  node.classList.toggle('is-done', done);
  node.classList.toggle('is-active', active);
  node.classList.toggle('is-failed', failed);
  const dot = node.querySelector('.installer-stage-dot');
  if (dot) {
    dot.textContent = '';
    if (done) dot.appendChild(icon('ui/check', { size: 13 }));
    else if (active) dot.appendChild(spinner({ size: 13 }));
    else dot.appendChild(el('span', 'installer-stage-idle'));
  }
  const stateNode = node.querySelector('.installer-stage-state');
  if (stateNode) {
    stateNode.textContent = failed
      ? T('progress.stage.failed', 'Failed')
      : done
        ? T('progress.stage.done', 'Done')
        : active
          ? T('progress.stage.active', 'In progress')
          : T('progress.stage.pending', 'Pending');
  }
}

function updateProgressDom() {
  const status = state.jobStatus;
  if (!status || !refs.ring) return;
  refs.ring.style.setProperty('--p', String(Math.max(0, Math.min(100, status.progress || 0))));
  if (refs.ringValue) refs.ringValue.textContent = `${status.progress || 0}%`;
  if (refs.progressStage) refs.progressStage.textContent = stageLabel(status.stage);
  if (refs.progressStatus) refs.progressStatus.textContent = status.error || T('progress.running', 'Working…');
  if (refs.log) refs.log.textContent = (status.log || []).join('\n');
  const current = stageIndex(status.stage);
  for (const [key, node] of Object.entries(refs.stages || {})) {
    paintStage(node, key, status, current);
  }
  renderFooter();
}

function progressHead() {
  const head = el('header', 'installer-step-head');
  const mark = el('div', 'installer-step-mark');
  mark.appendChild(icon('ui/download', { size: 22 }));
  const copy = el('div', 'installer-step-head-copy');
  const title = el('h2', 'installer-step-title', T('progress.title', "Installing PEAK'D!"));
  title.tabIndex = -1;
  copy.append(
    el('div', 'installer-step-kicker', 'INSTALL'),
    title,
    el('p', 'installer-step-subtitle', T('progress.body', 'The live session stays open while the helper prepares the target.')),
  );
  head.append(mark, copy);
  return head;
}

function renderProgress() {
  const status = state.jobStatus || {};
  const dryRun = !!state.health?.dry_run;
  const wrap = el('div', 'installer-progress');
  wrap.appendChild(progressHead());

  const hero = el('div', 'installer-progress-hero');
  const ring = el('div', 'installer-ring');
  ring.style.setProperty('--p', String(Math.max(0, Math.min(100, status.progress || 0))));
  refs.ring = ring;
  refs.ringValue = el('span', 'installer-ring-value', `${status.progress || 0}%`);
  ring.append(refs.ringValue, el('span', 'installer-ring-label', T('progress.completeShort', 'complete')));
  const copy = el('div', 'installer-progress-copy');
  copy.append(
    el('div', 'installer-progress-kicker', dryRun ? T('progress.dryBadge', 'Simulation') : T('progress.liveBadge', 'Install')),
    el('h3', 'installer-progress-title', T('progress.stepTitle', 'Preparing the target')),
    el('p', 'installer-progress-sub', T('progress.body', 'The live session stays open while the helper prepares the target.')),
  );
  refs.progressStage = el('div', 'installer-progress-stage', stageLabel(status.stage));
  refs.progressStatus = el('div', 'installer-progress-status', status.error || T('progress.running', 'Working…'));
  copy.append(refs.progressStage, refs.progressStatus);
  hero.append(ring, copy);
  wrap.appendChild(hero);

  if (status.done && status.success) {
    wrap.appendChild(note(
      'ok',
      'ui/check',
      dryRun ? T('progress.dryDoneTitle', 'Simulation complete') : T('progress.installDoneTitle', 'Installation complete'),
      dryRun
        ? T('progress.dryDoneBody', 'No disk was modified. The helper accepted this plan for testing.')
        : T('progress.installDoneBody', 'The target is ready. Restart the computer and remove the USB.'),
    ));
  } else if (status.done && !status.success) {
    wrap.appendChild(alertBlock('danger', T('progress.failed', 'Installation failed'), status.error || T('progress.failedBody', 'The installer stopped.')));
  } else if (dryRun) {
    wrap.appendChild(note('warn', 'ui/warning', T('progress.dryNotice', 'Simulation: no writes'), T('progress.dryNoticeBody', 'The helper is simulating the plan. No partition table is modified.')));
  }

  const stages = el('div', 'installer-stage-list');
  refs.stages = {};
  const current = stageIndex(status.stage);
  for (const [key, labelKey, fallback] of STAGE_KEYS) {
    const node = el('div', 'installer-stage');
    const dot = el('span', 'installer-stage-dot');
    const copyStage = el('span', 'installer-stage-copy');
    copyStage.append(el('span', 'installer-stage-label', T(labelKey, fallback)), el('span', 'installer-stage-state', ''));
    node.append(dot, copyStage);
    refs.stages[key] = node;
    paintStage(node, key, status, current);
    stages.appendChild(node);
  }
  wrap.appendChild(stages);

  const logWrap = el('section', 'installer-log-wrap');
  logWrap.appendChild(sectionHeading(T('progress.log', 'Live log'), T('progress.logHint', 'The helper reports each stage here.')));
  refs.log = el('pre', 'installer-log');
  refs.log.textContent = (status.log || []).join('\n') || T('progress.noLog', 'Waiting for the helper…');
  logWrap.appendChild(refs.log);
  wrap.appendChild(logWrap);
  refs.inner.appendChild(wrap);
}

async function cancelJob() {
  if (!state.job) return;
  try {
    await apiFetch('/api/installer/cancel', {
      method: 'POST',
      body: JSON.stringify({ job: state.job }),
    });
    toast(T('progress.cancelling', 'Cancelling…'), { type: 'info' });
  } catch (error) {
    toast(error.message, { type: 'error' });
  }
}

function backToReview() {
  state.phase = 'wizard';
  state.job = null;
  state.jobStatus = null;
  state.jobError = '';
  state.validation = null;
  state.validationKey = null;
  state.step = 4;
  render();
  refs.scroll.scrollTop = 0;
}

function renderManifest() {
  const manifest = refs.manifest;
  clear(manifest);
  const head = el('div', 'installer-manifest-head');
  const headCopy = el('div', 'installer-manifest-head-copy');
  headCopy.append(
    el('span', 'installer-eyebrow', T('manifest.eyebrow', 'Live plan')),
    el('h2', 'installer-manifest-title', T('manifest.title', 'Install manifest')),
  );
  head.appendChild(headCopy);
  head.appendChild(state.health?.dry_run ? badge(T('manifest.dryRun', 'Dry run'), { tone: 'accent' }) : badge(T('manifest.live', 'Live'), { tone: 'ok' }));
  manifest.appendChild(head);

  const disk = selectedDisk();
  const target = el('section', 'installer-manifest-section');
  target.appendChild(el('span', 'installer-manifest-label', T('manifest.target', 'Target')));
  const model = disk ? buildPlanModel(disk, state) : null;
  if (disk) {
    const root = selectedExistingRoot();
    const esp = selectedExistingEsp();
    target.append(
      el('strong', 'installer-manifest-value', disk.model || disk.path),
      el('span', 'installer-manifest-sub', `${disk.path} · ${formatBytes(disk.size_bytes)}`),
      segmentBar(existingSegments(disk), { mini: true, label: T('disk.currentBar', 'Current partition layout for {path}', { path: disk.path }) }),
      el('span', 'installer-manifest-meta', disk.serial || T('disk.noSerial', 'No serial exposed')),
    );
    if (model?.gap) {
      target.appendChild(el('span', 'installer-manifest-sub', `${T('disk.timeline.zone', 'Install zone')}: ${formatBytes(model.gap.start_bytes)} – ${formatBytes(model.gap.start_bytes + model.gap.size_bytes)}`));
    }
    if (state.partitionMode === MODE_EXISTING) {
      target.appendChild(el('span', 'installer-manifest-sub', `${T('disk.existing.rootTitle', 'Root partition')}: ${root?.path || '—'}`));
      target.appendChild(el('span', 'installer-manifest-sub', `${T('disk.existing.esp', 'EFI system')}: ${esp?.path || '—'}`));
    }
  } else {
    target.appendChild(el('span', 'installer-manifest-empty', T('manifest.noTarget', 'No disk selected')));
  }
  manifest.appendChild(target);

  const layoutSection = el('section', 'installer-manifest-section');
  layoutSection.appendChild(el('span', 'installer-manifest-label', T('manifest.layout', 'Layout')));
  layoutSection.appendChild(el('strong', 'installer-manifest-value', modeLabel()));
  if (model && state.partitionMode === MODE_EXISTING) {
    layoutSection.appendChild(el('span', 'installer-manifest-sub', `${T('review.rootPartition', 'Root partition')}: ${selectedExistingRoot()?.path || '—'}`));
    layoutSection.appendChild(el('span', 'installer-manifest-sub', `${T('review.preserved', 'Preserved partitions')}: ${model.kept.length}`));
  } else if (model) {
    layoutSection.appendChild(el('span', 'installer-manifest-sub', `${T('disk.efi', 'EFI system')} ${formatBytes(model.sizes.efi)} · ${T('disk.swapSegment', 'Swap')} ${formatBytes(model.sizes.swap)}`));
    layoutSection.appendChild(el('span', 'installer-manifest-sub', `${T('disk.root', "PEAK'D root")} ${formatBytes(model.sizes.root)}${model.sizes.home ? ` · ${T('disk.home', 'Home')} ${formatBytes(model.sizes.home)}` : ''}`));
    if (model.remaining > 0) {
      layoutSection.appendChild(el('span', 'installer-manifest-sub', `${T('disk.summary.freeAfter', 'Free space after install')}: ${formatBytes(model.remaining)}`));
    }
  }
  layoutSection.appendChild(segmentBar(plannedSegments(), { mini: true, label: T('disk.afterBar', 'Planned partition layout') }));
  manifest.appendChild(layoutSection);

  const system = el('section', 'installer-manifest-section');
  system.appendChild(el('span', 'installer-manifest-label', T('manifest.system', 'System')));
  system.appendChild(el('strong', 'installer-manifest-value', profileLabel()));
  system.appendChild(el('span', 'installer-manifest-sub', aiLabel()));
  system.appendChild(el('span', 'installer-manifest-sub', `${T('review.gpu', 'GPU driver')}: ${gpuLabel()}`));
  system.appendChild(el('span', 'installer-manifest-sub', `${T('review.cuda', 'CUDA')}: ${state.gpuDriver === 'nvidia' && state.installCuda ? T('common.yes', 'Yes') : T('common.no', 'No')}`));
  manifest.appendChild(system);

  if (acceptedLicenseRows().length) {
    const licenses = el('section', 'installer-manifest-section');
    licenses.appendChild(el('span', 'installer-manifest-label', T('manifest.licenses', 'Licenses')));
    for (const [id, detail] of acceptedLicenseRows()) {
      licenses.appendChild(el('span', 'installer-manifest-sub', `${id} · ${detail}`));
    }
    manifest.appendChild(licenses);
  }

  const account = el('section', 'installer-manifest-section');
  account.appendChild(el('span', 'installer-manifest-label', T('manifest.account', 'Account')));
  account.appendChild(el('strong', 'installer-manifest-value', state.fullName || state.username || T('manifest.noAccount', 'Not set')));
  account.appendChild(el('span', 'installer-manifest-sub', state.username ? `@${state.username} · ${state.hostname}` : T('manifest.noAccountSub', 'First login not configured')));
  manifest.appendChild(account);

  manifest.appendChild(el('p', 'installer-manifest-note', T('manifest.note', 'Nothing is written until you press Install.')));
}

export function mountInstallerTile() {
  if (tileEl) return tileEl;
  ensureStylesheet();
  state = initialState();
  fieldUid = 0;
  tileEl = document.createElement('section');
  tileEl.className = 'tile installer-tile';
  tileEl.dataset.plugin = PLUGIN;

  refs = {
    chrome: el('div', 'installer-chrome'),
    shell: el('div', 'installer-shell'),
    steps: el('nav', 'installer-steps'),
    main: el('main', 'installer-main'),
    scroll: el('div', 'installer-step-scroll'),
    inner: el('div', 'installer-step-inner'),
    footer: el('footer', 'installer-footer'),
    manifest: el('aside', 'installer-manifest'),
    live: el('div', 'installer-sr-only'),
  };
  refs.chrome.dataset.windowBar = '';
  refs.steps.setAttribute('aria-label', T('steps.nav', 'Installation steps'));
  refs.live.setAttribute('aria-live', 'polite');
  refs.scroll.appendChild(refs.inner);
  refs.main.append(refs.scroll, refs.footer);
  refs.shell.append(refs.steps, refs.main, refs.manifest);
  tileEl.append(refs.chrome, refs.shell, refs.live);

  render();
  void refreshData();
  return tileEl;
}

export function unmountInstallerTile() {
  if (pollTimer) window.clearInterval(pollTimer);
  pollTimer = null;
  refs = {};
  tileEl?.remove();
  tileEl = null;
  state = null;
}

export function getInstallerTileElement() {
  return tileEl;
}

export function wireInstallerEvents() {}

export function installerContextMenu() {
  return [
    { type: 'item', label: T('menu.refresh', 'Refresh disks'), icon: 'ui/refresh', onClick: () => void refreshData() },
    { type: 'item', label: T('menu.review', 'Back to review'), icon: 'ui/check', onClick: () => goStep(4) },
  ];
}

export default {
  name: PLUGIN,
  icon: 'ui/download',
  mount: mountInstallerTile,
  unmount: unmountInstallerTile,
  getElement: getInstallerTileElement,
  wireEvents: wireInstallerEvents,
  contextMenu: installerContextMenu,
};
