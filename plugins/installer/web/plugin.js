import {
  avatar, badge, button, controlButton, emptyState, field, icon, iconButton, input,
  keyValue, select, spinner, toast, listReveal, rollNumber, decodeText,
  paneStack, compactToolbar,
} from '../../ui/index.js';
import { apiFetch } from '../../js/api.js';
import { t } from '../../js/i18n.js';
import {
  GIB, MIB,
  MODE_ERASE, MODE_EXISTING, MODE_MANUAL,
  applyPreset, buildPlanModel, chooseDefaults,
  diskGaps, diskOs, diskReasons as baseDiskReasons, diskSpans, findEsp,
  layoutSpecsForPlan, placementForPlan,
  rootCandidates,
} from './timeline-model.js';
import { osLabel, partitionLabel, renderTimelinePanel } from './timeline.js';

const PLUGIN = 'installer';

const STEPS = [
  { id: 'region', icon: 'ui/keyboard' },
  // The disk step explains itself through the before/after bars.
  { id: 'disk', icon: 'ui/monitor', subtitle: false },
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
  ['unpack', 'progress.stage.unpack', 'Copying the system'],
  ['configure', 'progress.stage.configure', 'Configuring'],
  ['bootloader', 'progress.stage.bootloader', 'Bootloader'],
  ['finish', 'progress.stage.finish', 'Finishing'],
];

const T = (key, fallback, params) => t(`plugin.installer.${key}`, params, fallback);

let tileEl = null;
let refs = {};
/** The pane stack over the shell: main (the floor) and the manifest. */
let panes = null;
/** The footer as a compact toolbar (ui/compact.js). */
let footerBar = null;
let pollTimer = null;
let state = null;
let fieldUid = 0;
/** The step whose rows last cascaded in (listReveal once per step entry). */
let revealedStep = null;
let revealedProgress = false;

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
    hostname: 'liteaura',
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
    gpuDriver: 'nouveau',
    installCuda: false,
    acceptedLicenses: {},
    mokAccepted: false,
    selectedDiskPath: '',
    confirmed: false,
    job: null,
    jobStatus: null,
    jobError: '',
  };
}

function selectedDisk() {
  return (state.probe?.disks || []).find((disk) => disk.path === state.selectedDiskPath) || null;
}

function diskReasons(disk, mode = state.partitionMode) {
  return baseDiskReasons(disk, mode, state.probe?.live_medium);
}

function reasonLabel(reason) {
  const values = {
    missing: T('disk.reason.missing', 'Disk not found'),
    removable: T('disk.reason.removable', 'Removable device'),
    readOnly: T('disk.reason.readOnly', 'Read-only device'),
    loop: T('disk.reason.loop', 'Virtual disk'),
    optical: T('disk.reason.optical', 'CD/DVD drive'),
    mounted: T('disk.reason.mounted', 'A partition is mounted'),
    live: T('disk.reason.live', 'This is the drive LiteAura is running from'),
    serial: T('disk.reason.serial', 'No serial number reported'),
    small: T('disk.reason.small', 'Smaller than 8 GiB'),
    table: T('disk.reason.table', 'Uses an old partition table (only GPT is supported)'),
  };
  return values[reason] || reason;
}

function plannedBarLabel(kind) {
  if (kind === 'efi') return T('disk.efi', 'Startup');
  if (kind === 'swap') return T('disk.swapSegment', 'Support memory');
  if (kind === 'home') return T('disk.home', 'Personal files');
  return T('disk.root', "LiteAura system");
}

function modelBarSegment(segment) {
  let label = segment.label;
  if (segment.planned) label = plannedBarLabel(segment.kind);
  else if (segment.kind === 'free') label = T('disk.free', 'Free space');
  else if (segment.selectedRoot) label = T('disk.root', "LiteAura system");
  else if (segment.reused) label = T('disk.efi', 'Startup');
  else if (segment.part) label = partitionLabel(segment.part, T);
  // Same colour language as the timeline: existing partitions are grey,
  // whatever they hold; only LiteAura's own pieces are coloured.
  let kind = 'kept';
  if (segment.planned) kind = segment.kind;
  else if (segment.kind === 'free') kind = 'free';
  else if (segment.selectedRoot) kind = 'root';
  else if (segment.reused) kind = 'efi';
  return {
    kind,
    size: segment.size,
    label: label || segment.kind,
    detail: segment.part?.path || (segment.kind === 'free' ? T('disk.freeDetail', 'Not part of any partition') : ''),
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
        state.hostname = value ? `${value}-liteaura` : 'liteaura';
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

  if (!state.locale.trim()) add('locale', T('region.locale.required', 'Choose a language and formats'));
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

  if (!state.profile) add('profile', T('profile.required', 'Choose a system version'));
  if (!['off', 'none', 'voice', 'local'].includes(state.aiTier)) add('aiTier', T('profile.ai.required', 'Choose an AI tier'));

  if (state.gpuDriver === 'nvidia') {
    if (!state.acceptedLicenses[NVIDIA_DRIVER_LICENSE.id]) {
      add('gpuLicense', T('profile.nvidia.license.required', 'Accept the NVIDIA driver license'));
    }
    if (state.installCuda && !state.acceptedLicenses[CUDA_LICENSE.id]) {
      add('cudaLicense', T('profile.nvidia.cudaLicense.required', 'Accept the CUDA EULA'));
    }
    if (state.health?.hardware?.secure_boot && !state.mokAccepted) {
      add('mok', T('profile.nvidia.mok.required', 'Confirm that you understand the security-key step'));
    }
  }

  return errors;
}

function modelErrorLabel(code) {
  const values = {
    noDisk: T('disk.noDisk', 'No disk selected'),
    noFree: T('disk.manual.noFree', 'No usable free space found on this disk'),
    noOffsets: T('disk.timeline.noOffsets', 'Partition positions are approximate; refresh the disks before changing the layout.'),
    gapTooSmall: T('disk.timeline.gapTooSmall', 'The free space is too small for LiteAura.'),
    swapTooBig: T('disk.timeline.swapTooBig', 'The support memory does not fit in this free space.'),
    homeTooBig: T('disk.manual.homeTooBig', 'Leave at least 20 GiB for the LiteAura system'),
    rootTooBig: T('disk.timeline.rootTooBig', 'The system does not fit in this free space.'),
    rootSmall: T('disk.manual.rootSmall', 'Leave at least 20 GiB for the system. Reduce the personal-files size or the support memory.'),
    noRoot: T('disk.existing.noRoot', 'Choose a partition for LiteAura (ext4 or empty, not in use)'),
    noEsp: T('disk.existing.noEsp', 'No startup partition found on this disk'),
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
    if (!root) errors.push(T('disk.existing.noRoot', 'Choose a partition for LiteAura (ext4 or empty, not in use)'));
    if (!esp) errors.push(T('disk.existing.noEsp', 'No startup partition found on this disk'));
    if (root) {
      if ((root.mountpoints || []).length) {
        errors.push(T('disk.existing.rootMounted', 'The selected partition is in use'));
      }
      if ((root.holders || []).length) {
        errors.push(T('disk.existing.rootBusy', 'The selected partition is being used by the system'));
      }
      if (root.fstype && root.fstype !== 'ext4') {
        errors.push(T('disk.existing.rootFilesystem', 'The partition for LiteAura must be ext4 or empty'));
      }
      if (String(root.fstype || '').trim() && !state.existingConfirmed) {
        errors.push(T('disk.existing.needConfirm', 'Confirm that this partition will be formatted'));
      }
    }
    if (esp && esp.fstype !== 'vfat') {
      errors.push(T('disk.existing.espFilesystem', 'The startup partition must be FAT32'));
    }
    if (root && esp && root.path === esp.path) {
      errors.push(T('disk.existing.samePartition', 'The system and startup partitions must be different'));
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
    install_voice: state.aiTier === 'voice' || state.aiTier === 'local',
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
  if (tier === 'off') return T('profile.ai.off.title', 'No AI');
  if (tier === 'none') return T('profile.ai.none.title', 'Text only');
  if (tier === 'local') return T('profile.ai.local.title', 'Local AI');
  return T('profile.ai.voice.title', 'Voice pack');
}

function layoutLabel(preset = state.layoutPreset) {
  if (preset === 'simple') return T('disk.preset.simple.title', 'Minimal');
  if (preset === 'home') return T('disk.preset.home.title', 'Separate home');
  return T('disk.preset.swap.title', 'Recommended');
}

/** Same words as the choice cards on the disk step. */
function modeLabel(mode = state.partitionMode) {
  if (mode === MODE_EXISTING) return T('disk.intent.replace', 'Replace a partition');
  if (mode === MODE_MANUAL) {
    const os = diskOs(selectedDisk());
    return os
      ? T('disk.intent.beside', 'Next to {os}', { os: osLabel(os, T) })
      : T('disk.intent.free', 'Use the free space');
  }
  return T('disk.intent.erase', 'Erase the whole disk');
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
  if (stage === 'failed') return T('progress.failed', 'Installation failed');
  const entry = STAGE_KEYS.find(([key]) => key === stage);
  return entry ? T(entry[1], entry[2]) : stage;
}

function stageIndex(stage) {
  return STAGE_KEYS.findIndex(([key]) => key === stage);
}

/** "02 / 05": the zero-padded mono counter used by the rail and step heads. */
function counter(current, total) {
  return `${String(current).padStart(2, '0')} / ${String(total).padStart(2, '0')}`;
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
  // One mono status line: "LIVE SESSION · DRY RUN · STEP 2/5" (or the helper state).
  const status = el('div', 'installer-chrome-status ui-meta');
  const parts = [];
  let tone = 'idle';
  if (state.health) {
    tone = 'ok';
    parts.push(T('chrome.live', 'Live session'));
    if (state.health.dry_run) parts.push(T('chrome.dryRun', 'Test mode'));
    const totalSteps = state.phase === 'progress' ? STEPS.length + 1 : STEPS.length;
    const currentStep = state.phase === 'progress' ? totalSteps : Math.min(state.step + 1, STEPS.length);
    parts.push(T('chrome.stepShort', 'Step {current}/{total}', { current: currentStep, total: totalSteps }));
  } else if (state.loading) {
    parts.push(T('chrome.connecting', 'Connecting to the installer service'));
  } else {
    tone = 'error';
    parts.push(T('chrome.offline', 'Installer service offline'));
  }
  status.append(el('span', `installer-status-dot installer-status-dot--${tone}`), el('span', null, parts.join(' · ')));
  chrome.append(
    el('span', 'installer-chrome-spacer'),
    el('span', 'installer-chrome-title ui-meta', T('chrome.installer', 'Installer')),
    el('span', 'installer-chrome-spacer'),
    status,
  );
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
    // Flat rail: a mono number (a check once done) and the label.
    const num = el('span', 'installer-step-num');
    if (done) num.appendChild(icon('ui/check', { size: 12 }));
    else num.textContent = String(index + 1).padStart(2, '0');
    item.append(num, el('span', 'installer-step-label', label));
    list.appendChild(item);
  });

  if (state.phase === 'progress') {
    const item = el('div', 'installer-step-item is-active is-installing');
    item.setAttribute('aria-current', 'step');
    const num = el('span', 'installer-step-num');
    num.appendChild(spinner({ size: 12 }));
    item.append(num, el('span', 'installer-step-label', T('steps.progress.label', 'Installing')));
    list.appendChild(item);
  }
  nav.appendChild(list);
  requestAnimationFrame(() => {
    const active = list.querySelector('.installer-step-item.is-active');
    if (!active) return;
    // As a strip (stacked) the rail itself scrolls to centre the step: a
    // scrollIntoView would also drag the shell sideways, towards the
    // manifest pane waiting off to the right.
    if (nav.scrollWidth > nav.clientWidth && typeof active.getBoundingClientRect === 'function') {
      const a = active.getBoundingClientRect();
      const b = nav.getBoundingClientRect();
      nav.scrollLeft += (a.left + a.width / 2) - (b.left + b.width / 2);
      return;
    }
    active.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
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
  wrap.append(spinner({ size: 22 }), el('p', 'installer-loading-text', T('state.loading', 'Reading this computer and connecting to the installer service…')));
  refs.inner.appendChild(wrap);
}

function renderHelperError() {
  const wrap = el('div', 'installer-state');
  wrap.appendChild(emptyState({
    icon: 'ui/warning',
    title: T('state.helperDown.title', 'The installer service is not running'),
    body: state.healthError || T('state.helperDown.body', 'The installer works through a background service (shiny-installer-helper) that is not running. Restart the computer from the USB stick, then press Retry.'),
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
  const kicker = el('div', 'installer-step-kicker', index >= 0 ? counter(index + 1, STEPS.length) : 'INSTALL');
  const title = el('h2', 'installer-step-title', T(`steps.${step.id}.title`, step.id));
  title.tabIndex = -1;
  head.append(kicker, title);
  if (step.subtitle !== false) {
    head.appendChild(el('p', 'installer-step-subtitle', T(`steps.${step.id}.subtitle`, '')));
  }
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
  // Rows cascade in once per step entry, not on every re-render of the step.
  if (revealedStep !== step.id) {
    revealedStep = step.id;
    void listReveal(wrap.querySelectorAll(
      '.installer-disk, .installer-intent, .installer-choice, .installer-partition, .installer-review-block',
    ));
  }
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
      label: T('region.locale.label', 'Language and formats'),
      hint: T('region.locale.hint', 'Languages and formats installed on this system.'),
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
    T('region.live.title', 'You can keep using this session'),
    T('region.live.body', "LiteAura keeps running from the USB stick while you set up. Nothing is written to the disk until you press Install."),
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
      title: T('disk.empty.title', 'No disk available for installation'),
      body: T('disk.empty.body', 'The drive LiteAura is running from, virtual disks and CD/DVD drives are hidden. Plug in a disk or refresh.'),
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
  const pending = state.partitionMode === MODE_EXISTING && state.pendingExistingPath && !state.existingConfirmed;
  wrap.appendChild(renderTimelinePanel({
    disk,
    state,
    model,
    t: T,
    formatBytes,
    icon,
    confirm: pending ? renderExistingConfirm(disk) : null,
    onSelectGap: selectGap,
    onSelectPartition: selectExistingPartition,
    onToggle: toggleLayoutPart,
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

  const notes = renderGuidedNotes(disk);
  if (notes.children.length) wrap.appendChild(notes);
  return wrap;
}

function updateTimelineSummaryState() {
  if (refs.live) {
    refs.live.textContent = T('disk.timeline.updated', 'Disk layout updated');
  }
}

/** Usable disks up front; the ones that cannot be used fold into one line. */
function renderDiskShelf(disks) {
  const wrap = el('section', 'installer-disks');
  const bySize = (a, b) => Number(b.size_bytes) - Number(a.size_bytes);
  // Grouped by the least strict mode, so a disk does not jump between groups
  // when the user switches between "next to", "erase" and "replace".
  const usable = disks.filter((disk) => !diskReasons(disk, MODE_MANUAL).length).sort(bySize);
  const blocked = disks.filter((disk) => diskReasons(disk, MODE_MANUAL).length).sort(bySize);

  const shelf = el('div', 'installer-diskshelf');
  shelf.setAttribute('role', 'radiogroup');
  shelf.setAttribute('aria-label', T('disk.device', 'Installation disk'));
  for (const disk of usable) shelf.appendChild(diskCard(disk));
  if (usable.length) wrap.appendChild(shelf);

  if (blocked.length) {
    const details = el('details', 'installer-disks-blocked');
    if (!usable.length || blocked.some((disk) => disk.path === state.selectedDiskPath)) details.open = true;
    details.appendChild(el('summary', 'installer-disks-blocked-summary', T('disk.blocked', 'Not usable ({count})', {
      count: blocked.length,
    })));
    const list = el('div', 'installer-diskshelf installer-diskshelf--blocked');
    list.setAttribute('role', 'radiogroup');
    list.setAttribute('aria-label', T('disk.blocked', 'Not usable ({count})', { count: blocked.length }));
    for (const disk of blocked) list.appendChild(diskCard(disk));
    details.appendChild(list);
    wrap.appendChild(details);
  }
  return wrap;
}

/**
 * A tiny "before → after" drawing for each choice, in the same colours as the
 * big bars: grey = kept, hatched = free, red = erased, accent = LiteAura.
 */
const INTENT_PICTOS = {
  [MODE_MANUAL]: [[['kept', 3], ['free', 2]], [['kept', 3], ['new', 2]]],
  [MODE_ERASE]: [[['doomed', 3], ['doomed', 2]], [['new', 5]]],
  [MODE_EXISTING]: [[['kept', 2], ['doomed', 1.6], ['kept', 1.4]], [['kept', 2], ['new', 1.6], ['kept', 1.4]]],
};

function intentPicto(mode) {
  const picto = el('span', 'installer-picto');
  picto.setAttribute('aria-hidden', 'true');
  const [before, after] = INTENT_PICTOS[mode];
  const bar = (segments) => {
    const node = el('span', 'installer-picto-bar');
    for (const [kind, weight] of segments) {
      const seg = el('span', `installer-picto-seg installer-picto-seg--${kind}`);
      seg.style.flex = `${weight} 1 0`;
      node.appendChild(seg);
    }
    return node;
  };
  picto.append(bar(before), el('span', 'installer-picto-arrow', '→'), bar(after));
  return picto;
}

function renderIntentSelector(disk) {
  const choices = el('div', 'installer-intent-row');
  choices.setAttribute('role', 'radiogroup');
  choices.setAttribute('aria-label', T('disk.intent.title', 'What should happen to the files?'));

  const gaps = diskGaps(disk).filter((gap) => gap.eligible);
  const roots = rootCandidates(disk);
  const esp = findEsp(disk);
  const os = diskOs(disk);
  const options = [
    {
      mode: MODE_MANUAL,
      iconName: 'ui/layers',
      title: os
        ? T('disk.intent.beside', 'Next to {os}', { os: osLabel(os, T) })
        : T('disk.intent.free', 'Use the free space'),
      why: T('disk.intent.free.noneShort', 'No free space'),
      disabled: !gaps.length,
    },
    {
      mode: MODE_ERASE,
      iconName: 'ui/trash',
      title: T('disk.intent.erase', 'Erase the whole disk'),
      why: diskReasons(disk, MODE_ERASE).map(reasonLabel).join(' · '),
      disabled: diskReasons(disk, MODE_ERASE).length > 0,
      danger: true,
    },
    {
      mode: MODE_EXISTING,
      iconName: 'ui/refresh',
      title: T('disk.intent.replace', 'Replace a partition'),
      why: T('disk.intent.partition.noneShort', 'No suitable partition'),
      disabled: !roots.length || !esp,
    },
  ];

  for (const option of options) {
    const selected = state.partitionMode === option.mode;
    const card = el('button', `installer-intent${selected ? ' is-selected' : ''}${option.danger ? ' is-danger' : ''}`);
    card.type = 'button';
    card.disabled = option.disabled;
    card.setAttribute('role', 'radio');
    card.setAttribute('aria-checked', String(selected));
    card.addEventListener('click', () => setPartitionMode(option.mode));
    const head = el('span', 'installer-intent-head');
    const mark = el('span', 'installer-intent-icon');
    mark.appendChild(icon(option.iconName, { size: 16 }));
    head.append(mark, el('span', 'installer-intent-title', option.title));
    card.append(head, intentPicto(option.mode));
    if (option.disabled && option.why) {
      card.appendChild(el('span', 'installer-intent-why', option.why));
    }
    choices.appendChild(card);
  }
  return choices;
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
  wrap.setAttribute('role', 'alert');
  const mark = el('span', 'installer-existing-mark');
  mark.appendChild(icon('ui/warning', { size: 15 }));
  const name = part ? `${partitionLabel(part, T)} ${formatBytes(part.size_bytes)}` : state.pendingExistingPath;
  wrap.append(mark, el('span', 'installer-existing-text', T('disk.existing.confirmShort', 'Everything in {name} will be erased', { name })));
  const actions = el('div', 'installer-existing-actions');
  actions.append(
    button({
      label: T('common.cancel', 'Cancel'),
      variant: 'ghost',
      icon: 'ui/close',
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
  state.confirmed = false;
  state.showErrors = false;
  invalidateValidation();
  renderMain();
  renderFooter();
  renderManifest();
}

/** Only the notes that block the install; everything else is on the bars. */
function renderGuidedNotes(disk) {
  const wrap = el('div', 'installer-guided-notes');
  const roots = rootCandidates(disk);
  const gaps = diskGaps(disk).filter((gap) => gap.eligible);
  if (!gaps.length && !roots.length && (disk.children || []).length) {
    wrap.appendChild(note(
      'warn',
      'ui/warning',
      T('disk.guided.noSpaceTitle', 'Not enough space for LiteAura'),
      T('disk.guided.noSpaceBody', 'This disk has no usable free space and no partition LiteAura can reuse. Erase the disk or free some space from the other system, then refresh.'),
    ));
  }
  if (!findEsp(disk) && state.partitionMode === MODE_EXISTING) {
    wrap.appendChild(alertBlock(
      'danger',
      T('disk.existing.noEspTitle', 'Startup partition missing'),
      T('disk.existing.noEspBody', 'This disk has no FAT32 startup partition. LiteAura needs one to start; choose “Create the partitions” or erase the whole disk.'),
    ));
  }
  return wrap;
}

function diskCard(disk) {
  const active = disk.path === state.selectedDiskPath;
  const blocked = diskReasons(disk, MODE_MANUAL);
  const card = controlButton();
  card.type = 'button';
  card.className = `installer-disk${active ? ' is-selected' : ''}${blocked.length ? ' has-reasons' : ''}`;
  card.setAttribute('role', 'radio');
  card.setAttribute('aria-checked', String(active));
  card.addEventListener('click', () => selectDisk(disk.path));
  // Technical identity stays one hover away; the review step shows it in full.
  card.title = [disk.path, disk.serial, disk.tran ? disk.tran.toUpperCase() : ''].filter(Boolean).join(' · ');

  const main = el('div', 'installer-disk-main');
  const title = el('div', 'installer-disk-title');
  title.append(
    el('span', 'installer-disk-name', disk.model || T('disk.unknownModel', 'Disk')),
    el('span', 'installer-disk-size', formatBytes(disk.size_bytes)),
  );
  main.appendChild(title);
  main.appendChild(segmentBar(existingSegments(disk), { mini: true, label: T('disk.currentBar', 'Current partition layout for {path}', { path: disk.path }) }));
  if (blocked.length) {
    main.appendChild(el('span', 'installer-disk-why', blocked.map(reasonLabel).join(' · ')));
  }

  const check = el('span', 'installer-disk-check');
  if (active) check.appendChild(icon('ui/check', { size: 14 }));
  card.append(main, check);
  return card;
}

function selectDisk(path) {
  if (state.selectedDiskPath === path) return;
  state.selectedDiskPath = path;
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

/**
 * The two switches under the disk bars. Swap and a separate /home map onto
 * the presets the plan already knows; sizes are then dragged on the bar.
 */
function toggleLayoutPart(kind, on) {
  const disk = selectedDisk();
  if (!disk) return;
  const model = buildPlanModel(disk, state);
  const swapOn = kind === 'swap' ? on : model.sizes.swap > 0;
  const homeOn = kind === 'home' ? on : model.sizes.home > 0;
  const preset = homeOn ? 'home' : swapOn ? 'swap' : 'simple';
  const rawBytes = state.partitionMode === MODE_MANUAL
    ? (model.gap?.size_bytes || 0)
    : disk.size_bytes;
  applyPreset(state, state.partitionMode, preset, rawBytes);
  if (!swapOn) {
    if (state.partitionMode === MODE_MANUAL) state.manualSwapGiB = 0;
    else state.swapGiB = 0;
  }
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
    badge(T('account.owner', "LiteAura owner"), { tone: 'accent' }),
    badge(T('account.admin', 'Administrator'), { tone: 'neutral' }),
  );
  wrap.appendChild(card);

  const fields = el('div', 'installer-account-fields');
  const fullName = createInput('fullName', { placeholder: 'Ada Lovelace', autocomplete: 'name' });
  const username = createInput('username', { placeholder: 'ada', autocomplete: 'username' });
  const hostname = createInput('hostname', { placeholder: 'liteaura', autocomplete: 'off' });
  refs.hostnameInput = hostname;
  fields.appendChild(fieldWithError({
    label: T('account.fullName.label', 'Full name'),
    hint: T('account.fullName.hint', "The name shown on the lock screen and in the LiteAura profile."),
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
      label: T('account.hostname.label', 'Computer name'),
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
    T('account.password.dryTitle', 'Your password'),
    T('account.password.dryBody', 'You will use this password to sign in. It is only used to create your account and is never saved in the installation log.'),
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
    el('span', 'installer-hardware-meta', [hw.vendor, hw.board].filter(Boolean).join(' · ') || T('profile.hardwareMeta', 'Hardware details unavailable')),
  );
  const badges = el('div', 'installer-hardware-badges');
  badges.appendChild(badge(hw.is_t2 ? T('profile.detectedT2', 'Apple T2 chip detected') : T('profile.detectedGeneric', 'Generic x86_64'), {
    tone: hw.is_t2 ? 'accent' : 'neutral',
  }));
  hardware.append(mark, copy, badges);
  wrap.appendChild(hardware);

  wrap.appendChild(sectionHeading(
    T('profile.kernel.title', 'System version'),
    T('profile.kernel.hint', 'Two system versions are on this USB stick. This choice decides which one the installed system uses.'),
  ));
  const profiles = el('div', 'installer-layout-choices');
  profiles.setAttribute('role', 'radiogroup');
  profiles.setAttribute('aria-label', T('profile.kernel.title', 'System version'));
  profiles.append(
    choiceCard({
      value: 'backports',
      current: state.profile === 'backports',
      iconName: 'ui/monitor',
      title: T('profile.generic.title', 'Generic PC'),
      desc: T('profile.generic.desc', 'For most PCs: an up-to-date kernel with current graphics and audio drivers.'),
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
      desc: T('profile.t2.desc', 'For Intel Macs with the Apple T2 chip: keyboard, trackpad, Wi-Fi and audio support.'),
      meta: hw.is_t2 ? T('profile.t2.meta', 'linux-t2 · detected') : T('profile.t2.missing', 'Apple T2 chip not detected'),
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
      T('profile.generic.warnTitle', 'Generic PC version on a T2 Mac'),
      T('profile.generic.warnBody', 'Keyboard, trackpad, Wi-Fi, Bluetooth or audio may not work. Choose the T2 Mac version for this computer.'),
    ));
  }
  wrap.appendChild(note(
    'accent',
    'ui/layers',
    T('profile.kernel.noteTitle', 'One USB stick, two system versions'),
    T('profile.kernel.noteBody', 'The Generic PC version installs the standard kernel. The T2 Mac version installs only the kernel for Macs with the T2 chip.'),
  ));

  wrap.appendChild(sectionHeading(
    T('profile.ai.title', 'AI tier'),
    T('profile.ai.hint', 'Choose how much of the local AI to install. You can add models later.'),
  ));
  const tiers = el('div', 'installer-layout-choices installer-layout-choices--ai');
  tiers.setAttribute('role', 'radiogroup');
  tiers.setAttribute('aria-label', T('profile.ai.title', 'AI tier'));
  const aiOptions = [
    ['off', 'ui/brain-off', T('profile.ai.off.title', 'No AI'), T('profile.ai.off.desc', 'No AI at all. The AI button opens the launcher.'), T('profile.ai.off.meta', 'You can turn it on later')],
    ['none', 'ui/keyboard', T('profile.ai.none.title', 'Text only'), T('profile.ai.none.desc', 'Chat and agent, no voice models.'), T('profile.ai.none.meta', 'Uses the least disk space')],
    ['voice', 'ui/download', T('profile.ai.voice.title', 'Voice pack'), T('profile.ai.voice.desc', 'Speech recognition and neural voices.'), T('profile.ai.voice.meta', 'Recommended')],
    ['local', 'ui/layers', T('profile.ai.local.title', 'Local AI'), T('profile.ai.local.desc', 'Voice plus local image and music models.'), T('profile.ai.local.meta', 'Needs more disk space')],
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
      desc: T('profile.nvidia.open.desc', 'Built into Linux (nouveau): no extra license, lower performance.'),
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
      T('profile.nvidia.datacenterBody', 'The standard NVIDIA driver is not offered for this GPU.'),
    ));
  }

  if (state.gpuDriver === 'nvidia') {
    wrap.appendChild(note(
      'warn',
      'ui/warning',
      T('profile.nvidia.warnTitle', 'Proprietary driver'),
      T('profile.nvidia.warnBody', 'The open driver (nouveau) is turned off and the system runs NVIDIA’s closed-source driver. You can switch back after installation.'),
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
      mokRow.append(mokCheck, el('span', 'installer-confirm-check-text', T('profile.nvidia.mok', 'I understand that at the first restart I must confirm a security key (MOK) on a setup screen')));
      wrap.appendChild(mokRow);
      wrap.appendChild(el('p', 'installer-hint', T('profile.nvidia.mokHint', 'With Secure Boot on, the NVIDIA driver must be signed with a key that you confirm once at the first restart.')));
    }
    if (nvidia.is_hybrid) {
      wrap.appendChild(note(
        'accent',
        'ui/info',
        T('profile.nvidia.hybridTitle', 'Hybrid graphics'),
        T('profile.nvidia.hybridBody', 'The integrated GPU stays on. LiteAura uses the NVIDIA GPU only for apps that need it.'),
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
      T('review.dryTitle', 'Test mode'),
      T('review.dryBody', 'The installer is running in test mode: pressing Install only rehearses the steps. No disk is changed.'),
    ));
  } else {
    wrap.appendChild(note(
      'danger',
      'ui/warning',
      T('review.realTitle', 'This will change the disk'),
      T('review.realBody', 'The partitions shown below will be formatted and their contents erased. Check the disk and the layout before continuing.'),
    ));
  }

  const reviewGrid = el('div', 'installer-review-grid');
  reviewGrid.appendChild(reviewBlock(T('review.region', 'Language & region'), keyValue([
    [T('region.locale.label', 'Language and formats'), state.locale],
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
    [T('account.hostname.label', 'Computer name'), state.hostname || '—'],
    [T('account.password.label', 'Password'), T('review.passwordValue', 'Set during install')],
  ])));

  const systemRows = [
    [T('profile.kernel.title', 'System version'), profileLabel()],
    [T('profile.ai.title', 'AI tier'), aiLabel()],
    [T('review.mode', 'Mode'), modeLabel()],
    [T('review.gpu', 'GPU driver'), gpuLabel()],
    [T('review.cuda', 'CUDA'), state.gpuDriver === 'nvidia' && state.installCuda ? T('common.yes', 'Yes') : T('common.no', 'No')],
  ];
  if (state.health?.hardware?.secure_boot && state.gpuDriver === 'nvidia') {
    systemRows.push([T('review.mok', 'Secure Boot key (MOK)'), state.mokAccepted ? T('common.acknowledged', 'Acknowledged') : T('common.required', 'Required')]);
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
    validating.append(spinner({ size: 16 }), el('span', null, T('review.validating', 'Checking the disk…')));
    wrap.appendChild(validating);
  } else if (state.validationError) {
    wrap.appendChild(alertBlock('danger', T('review.validationFailed', 'Disk check failed'), state.validationError));
  } else if (state.validation) {
    const errorsList = state.validation.errors || [];
    const warnings = state.validation.warnings || [];
    if (!errorsList.length && !warnings.length) {
      wrap.appendChild(note(
        'ok',
        'ui/check',
        T('review.valid', 'Disk checked'),
        T('review.validBody', 'The installer confirmed this is the right disk and the plan is ready.'),
      ));
    }
    if (errorsList.length) {
      wrap.appendChild(alertBlock('danger', T('review.errors', 'The disk needs attention'), errorsList.join(' · ')));
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
    el('h3', 'installer-confirm-title', T('review.confirm.title', 'Confirm the disk')),
    el('p', 'installer-confirm-body', T('review.confirm.body', 'Read what will happen to this disk, then tick the box to confirm.')),
  );
  confirmHead.append(confirmMark, confirmCopy);
  confirm.appendChild(confirmHead);

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
  return state.confirmed;
}

function renderFooter() {
  // The footer is a compact toolbar (its More menu is its own child); the
  // row inside it is what gets redrawn.
  const footer = refs.footerRow;
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
    })} · ${T('review.notReadyShort', 'Confirmation required')}`;
  } else {
    step.textContent = T('common.step', 'Step {current} of {total}', {
      current: Math.min(state.step + 1, STEPS.length),
      total: STEPS.length,
    });
  }
  footer.appendChild(step);
  if (refs.live) refs.live.textContent = step.textContent;

  // While the manifest is a pane over the main (a narrow window, a phone)
  // the summary opens from here; wide, the aside is already beside the step.
  footer.appendChild(iconButton({
    icon: 'ui/info',
    size: 'sm',
    label: T('manifest.title', 'Installation summary'),
    className: 'installer-manifest-open',
    onClick: () => panes?.show('manifest'),
  }));

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
        icon: 'ui/close',
        onClick: () => void cancelJob(),
      }));
    }
  } else {
    actions.appendChild(button({
      label: T('common.back', 'Back'),
      variant: 'quiet',
      icon: 'ui/chevron-left',
      disabled: state.step === 0,
      onClick: () => goStep(state.step - 1),
    }));
    if (state.step < STEPS.length - 1) {
      actions.appendChild(button({
        label: T('common.next', 'Next'),
        variant: 'ghost',
        icon: 'ui/chevron-right',
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
        label: T('common.install', "Install LiteAura"),
        variant: 'primary',
        icon: 'ui/download',
        disabled: !installReady(),
        onClick: () => void startInstall(),
      });
      if (!installReady()) {
        install.title = T('review.notReady', 'Complete the highlighted fields and tick the confirmation box');
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
    toast(T('review.noDisk', 'No disk selected.'), { type: 'error' });
    return;
  }
  if (!state.confirmed) {
    toast(T('review.needConfirm', 'Tick the confirmation box first'), { type: 'error' });
    return;
  }
  if (!state.validation?.ok || state.validationKey !== planKey()) {
    await validatePlan();
    if (!state.validation?.ok) {
      toast(T('review.targetNotValid', 'The installer has not checked this disk yet'), { type: 'error' });
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
        // The helper re-probes the device and refuses unless this is still
        // the disk the plan names (model, serial, size): the identity check
        // is the helper's, so the user is not asked to type the serial.
        confirm_serial: disk.serial || '',
      }),
    });
    if (!tileEl) return;
    state.job = response?.data?.job || null;
    if (!state.job) throw new Error(T('progress.noJob', 'The installer could not start the installation'));
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
  // The stage label decodes in place when the helper moves to the next stage.
  if (refs.progressStage) {
    const label = stageLabel(status.stage);
    if (refs.progressStage.dataset.stage !== String(status.stage || '')) {
      refs.progressStage.dataset.stage = String(status.stage || '');
      decodeText(refs.progressStage, label);
    }
  }
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
  const title = el('h2', 'installer-step-title', T('progress.title', "Installing LiteAura"));
  title.tabIndex = -1;
  head.append(
    el('div', 'installer-step-kicker', counter(STEPS.length + 1, STEPS.length + 1)),
    title,
    el('p', 'installer-step-subtitle', T('progress.body', 'You can keep using this live session while LiteAura is installed on the disk.')),
  );
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
    el('div', 'installer-progress-kicker', dryRun ? T('progress.dryBadge', 'Test') : T('progress.liveBadge', 'Install')),
    el('h3', 'installer-progress-title', T('progress.stepTitle', 'Preparing the disk')),
    el('p', 'installer-progress-sub', T('progress.body', 'You can keep using this live session while LiteAura is installed on the disk.')),
  );
  refs.progressStage = el('div', 'installer-progress-stage', stageLabel(status.stage));
  refs.progressStage.dataset.stage = String(status.stage || '');
  refs.progressStatus = el('div', 'installer-progress-status', status.error || T('progress.running', 'Working…'));
  copy.append(refs.progressStage, refs.progressStatus);
  hero.append(ring, copy);
  wrap.appendChild(hero);

  if (status.done && status.success) {
    wrap.appendChild(note(
      'ok',
      'ui/check',
      dryRun ? T('progress.dryDoneTitle', 'Test complete') : T('progress.installDoneTitle', 'Installation complete'),
      dryRun
        ? T('progress.dryDoneBody', 'No disk was changed. The installer went through every step without writing.')
        : T('progress.installDoneBody', 'LiteAura is installed. Restart the computer and remove the USB stick.'),
    ));
  } else if (status.done && !status.success) {
    wrap.appendChild(alertBlock('danger', T('progress.failed', 'Installation failed'), status.error || T('progress.failedBody', 'The installation stopped before finishing. The log below shows the reason.')));
  } else if (dryRun) {
    wrap.appendChild(note('warn', 'ui/warning', T('progress.dryNotice', 'Test mode: nothing is written'), T('progress.dryNoticeBody', 'The installer is rehearsing the steps. No disk is changed.')));
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
  logWrap.appendChild(sectionHeading(T('progress.log', 'Installation log'), T('progress.logHint', 'Each step of the installation is shown here.')));
  refs.log = el('pre', 'installer-log');
  refs.log.textContent = (status.log || []).join('\n') || T('progress.noLog', 'Waiting for the installer service…');
  logWrap.appendChild(refs.log);
  wrap.appendChild(logWrap);
  refs.inner.appendChild(wrap);
  // The ring rolls up to its value and the stages cascade in when the view opens.
  if (!revealedProgress) {
    revealedProgress = true;
    rollNumber(refs.ringValue, { duration: 700 });
    void listReveal(stages.children);
  }
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
  revealedProgress = false;
  render();
  refs.scroll.scrollTop = 0;
}

function renderManifest() {
  // The head is the pane bar (built once at mount, the stack's Back lands
  // in it); only the body is redrawn.
  const manifest = refs.manifestBody;
  clear(manifest);

  const disk = selectedDisk();
  const target = el('section', 'installer-manifest-section');
  target.appendChild(el('span', 'installer-manifest-label', T('manifest.target', 'Disk')));
  const model = disk ? buildPlanModel(disk, state) : null;
  if (disk) {
    target.append(
      el('strong', 'installer-manifest-value', disk.model || disk.path),
      el('span', 'installer-manifest-sub', formatBytes(disk.size_bytes)),
    );
  } else {
    target.appendChild(el('span', 'installer-manifest-empty', T('manifest.noTarget', 'No disk selected')));
  }
  manifest.appendChild(target);

  // The plan as one picture: the "after" bar and how big LiteAura gets.
  const layoutSection = el('section', 'installer-manifest-section');
  layoutSection.appendChild(el('span', 'installer-manifest-label', T('manifest.layout', 'Layout')));
  layoutSection.appendChild(el('strong', 'installer-manifest-value', modeLabel()));
  if (model) {
    const rootBytes = state.partitionMode === MODE_EXISTING
      ? Number(selectedExistingRoot()?.size_bytes) || 0
      : model.sizes.root;
    if (rootBytes) layoutSection.appendChild(el('span', 'installer-manifest-sub', `LiteAura ${formatBytes(rootBytes)}`));
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
  revealedStep = null;
  revealedProgress = false;
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
    footerRow: el('div', 'installer-footer-row'),
    manifest: el('aside', 'installer-manifest'),
    manifestHead: el('div', 'installer-manifest-head'),
    manifestBody: el('div', 'installer-manifest-body'),
    live: el('div', 'installer-sr-only'),
  };
  refs.chrome.dataset.windowBar = '';
  refs.manifestHead.dataset.paneBar = '';
  const headCopy = el('div', 'installer-manifest-head-copy');
  headCopy.append(
    el('span', 'installer-eyebrow', T('manifest.eyebrow', 'Your plan')),
    el('h2', 'installer-manifest-title', T('manifest.title', 'Installation summary')),
  );
  refs.manifestHead.appendChild(headCopy);
  refs.manifest.append(refs.manifestHead, refs.manifestBody);
  refs.steps.setAttribute('aria-label', T('steps.nav', 'Installation steps'));
  refs.live.setAttribute('aria-live', 'polite');
  refs.scroll.appendChild(refs.inner);
  refs.footerRow.dataset.fold = '';
  refs.footer.appendChild(refs.footerRow);
  refs.main.append(refs.scroll, refs.footer);
  refs.shell.append(refs.steps, refs.main, refs.manifest);
  tileEl.append(refs.chrome, refs.shell, refs.live);

  // Rail · main · manifest side by side while wide; at 1020px or on a phone
  // the manifest is a pane that slides over the main from the footer's
  // info button and closes with Back / Escape, the rail a strip above.
  panes = paneStack(refs.shell, {
    panes: [{ id: 'main', el: refs.main }, { id: 'manifest', el: refs.manifest }],
    compactWhen: ({ width, phone }) => phone || width <= 1020,
  });
  footerBar = compactToolbar(refs.footer);

  render();
  void refreshData();
  return tileEl;
}

export function unmountInstallerTile() {
  if (pollTimer) window.clearInterval(pollTimer);
  pollTimer = null;
  panes?.destroy();
  panes = null;
  footerBar?.destroy();
  footerBar = null;
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
