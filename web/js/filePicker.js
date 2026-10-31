/**
 * filePicker.js — the shell's own file dialog.
 *
 * The shell's pickers and the Craft web apps use a classic `<input type=file>`.
 * In the kiosk the web engine has no file-dialog handler, so clicking Open did
 * nothing. This module is the shell side of the answer: it browses the user's
 * home through the Files plugin (`/api/files/home`, `/api/files/list`,
 * `/api/files/raw`) and returns the picked entries — or real `File` objects for
 * code that wants bytes.
 *
 * It is app-agnostic: the plugin upload, the wallpaper picker and every web-app
 * `<input type=file>` (through the shim in `webAppSurface.js`) use the same
 * dialog, and every Craft app installed later gets it for free.
 */

import { apiFetch } from './api.js';
import { floating, button, iconButton, icon, emptyState, toast } from '../ui/index.js';
import { t } from './i18n.js';

/** The Files plugin serves inline files up to this size; bigger reads use ranges. */
const MAX_PICK_BYTES = 256 * 1024 * 1024;

/** Cached answer to "can the shell offer its picker?" */
let available;

/**
 * Turn an `<input accept>` value into a predicate over Files entries.
 * Extensions (`.docx`), wildcard types (`image/*`) and common exact types are
 * understood; an unknown rule keeps the file rather than hiding something the
 * app may accept.
 */
export function acceptMatcher(accept) {
  const rules = String(accept ?? '')
    .split(',')
    .map((rule) => rule.trim().toLowerCase())
    .filter(Boolean);
  if (!rules.length) return () => true;
  return (entry) => {
    if (!entry || entry.kind === 'dir') return true;
    const ext = String(entry.ext ?? '').toLowerCase();
    const mime = String(entry.mime ?? '').toLowerCase();
    return rules.some((rule) => {
      if (rule.startsWith('.')) return rule.slice(1) === ext;
      if (rule.endsWith('/*')) {
        if (rule === 'image/*') return !!entry.is_image || mime.startsWith('image/');
        if (rule === 'audio/*') return !!entry.is_audio || mime.startsWith('audio/');
        if (rule === 'video/*') return !!entry.is_video || mime.startsWith('video/');
        if (rule === 'text/*') return !!entry.is_text || mime.startsWith('text/');
        return true;
      }
      if (rule === 'application/pdf') return !!entry.is_pdf || ext === 'pdf';
      if (rule === 'application/zip') return !!entry.is_archive || ext === 'zip';
      if (rule === 'text/plain') return !!entry.is_text || ext === 'txt';
      return true;
    });
  };
}

/** Raw bytes of one home-relative file (the Files plugin's own read route). */
export async function readFileBytes(path) {
  return apiFetch(`/api/files/raw?path=${encodeURIComponent(path)}`, { responseType: 'blob' });
}

/**
 * Turn picked entries into `File` objects **of the given realm**. A web app's
 * `File`/`DataTransfer` classes belong to its own window, so the shim builds
 * them there (`realm.File`) — mixing realms makes some engines reject the
 * assignment to `input.files`.
 */
export async function filesFromEntries(realm, entries) {
  const files = [];
  for (const entry of entries) {
    const blob = await readFileBytes(entry.path);
    files.push(new realm.File([blob], entry.name, { type: blob?.type || '' }));
  }
  return files;
}

/** Is the Files plugin answering? One probe per session. */
export async function pickerAvailable() {
  if (available !== undefined) return available;
  try {
    await apiFetch('/api/files/home');
    available = true;
  } catch {
    available = false;
  }
  return available;
}

/** The Files window's own icon mapping, so a file looks the same in both. */
function entryIcon(entry) {
  if (entry.kind === 'dir') return 'ui/folder';
  if (entry.is_image) return 'ui/image';
  if (entry.is_pdf) return 'ui/doc';
  if (entry.is_video) return 'ui/video';
  if (entry.is_audio) return 'ui/music';
  if (entry.is_archive) return 'ui/archive';
  return 'ui/file';
}

/** The coloured folder artwork the Files window uses for the home folders. */
const SPECIAL_FOLDERS = {
  desktop: 'ui/folder-desktop',
  documents: 'ui/folder-documents',
  downloads: 'ui/folder-downloads',
  music: 'ui/folder-music',
  pictures: 'ui/folder-pictures',
  public: 'ui/folder-public',
  templates: 'ui/folder-templates',
  videos: 'ui/folder-videos',
};

function folderIconFor(name) {
  return SPECIAL_FOLDERS[String(name || '').toLowerCase()] || 'ui/folder';
}

/** A live thumbnail for images, the Files artwork for everything else. */
function thumbFor(entry) {
  const thumb = document.createElement('span');
  thumb.className = 'files-thumb';
  if (entry.kind === 'dir') {
    thumb.classList.add('files-thumb--folder');
    thumb.appendChild(icon(folderIconFor(entry.name), { size: 24 }));
    return thumb;
  }
  if (entry.is_image) {
    const img = document.createElement('img');
    img.className = 'files-grid-img';
    img.alt = '';
    img.loading = 'lazy';
    const typeIcon = () => {
      img.remove();
      thumb.classList.add('files-thumb--icon');
      thumb.appendChild(icon(entryIcon(entry), { size: 24 }));
    };
    img.addEventListener('error', () => {
      if (img.dataset.fallback) {
        typeIcon();
        return;
      }
      img.dataset.fallback = '1';
      img.src = `/api/files/raw?path=${encodeURIComponent(entry.path)}`;
    });
    img.src = `/api/files/thumb?path=${encodeURIComponent(entry.path)}&size=320`;
    thumb.appendChild(img);
    return thumb;
  }
  thumb.classList.add('files-thumb--icon');
  thumb.appendChild(icon(entryIcon(entry), { size: 24 }));
  return thumb;
}

/**
 * One grid cell (the Files window's classes, so the grid reads the same):
 * click picks a file / opens a folder, double-click or Enter opens the file.
 */
function entryCell(entry, { onPick, onOpen }) {
  const cell = document.createElement('button');
  cell.type = 'button';
  cell.className = 'files-entry files-entry--grid file-picker-entry';
  cell.dataset.path = entry.path;
  cell.title = entry.name;
  const name = document.createElement('span');
  name.className = 'files-grid-name';
  name.textContent = entry.name;
  cell.append(thumbFor(entry), name);
  cell.addEventListener('click', (event) => {
    if (entry.kind === 'dir') onOpen(entry);
    else if (event.detail === 2) onOpen(entry);
    else onPick(entry);
  });
  cell.addEventListener('keydown', (event) => {
    if (event.key !== 'Enter') return;
    event.preventDefault();
    if (entry.kind === 'dir') onOpen(entry);
    else {
      onPick(entry);
      onOpen(entry);
    }
  });
  return cell;
}

/**
 * The picker window. Resolves the chosen entries (`{name, path, kind, ext,
 * size}`), an empty array on Cancel, and `null` when the Files plugin is not
 * available (callers then keep their old behavior).
 */
export async function pickPaths({ accept = '', multiple = false } = {}) {
  if (!(await pickerAvailable())) return null;
  const matches = acceptMatcher(accept);

  return new Promise((resolve) => {
    let current = '';
    let entries = [];
    let settled = false;
    const chosen = new Map();

    const pathLabel = document.createElement('div');
    pathLabel.className = 'file-picker-path';
    const upBtn = iconButton({
      icon: 'ui/arrow-up',
      label: t('files.pickerUp', null, 'Up one folder'),
      size: 'sm',
      variant: 'quiet',
    });
    const pathText = document.createElement('span');
    pathText.className = 'file-picker-path-text';
    pathLabel.append(upBtn, pathText);

    const gridHost = document.createElement('div');
    gridHost.className = 'files-grid file-picker-grid';
    const cells = new Map();

    const footer = document.createElement('div');
    footer.className = 'file-picker-actions';
    const summary = document.createElement('span');
    summary.className = 'file-picker-summary';
    const cancelBtn = button({
      label: t('files.pickerCancel', null, 'Cancel'),
      variant: 'ghost',
      size: 'sm',
    });
    const openBtn = button({
      label: t('files.pickerOpen', null, 'Open'),
      variant: 'primary',
      size: 'sm',
      disabled: true,
    });
    footer.append(summary, cancelBtn, openBtn);

    const dialog = floating({
      key: 'file-picker',
      title: t('files.pickerTitle', null, 'Open a file'),
      body: [pathLabel, gridHost, footer],
      removeOnClose: true,
      onClose: () => finish([]),
    });

    function finish(result) {
      if (settled) return;
      settled = true;
      resolve(result);
    }

    function close(result) {
      finish(result);
      dialog.close();
    }

    cancelBtn.addEventListener('click', () => close([]));
    openBtn.addEventListener('click', () => close([...chosen.values()]));
    upBtn.addEventListener('click', () => {
      if (!current) return;
      void load(current.includes('/') ? current.replace(/\/[^/]+$/, '') : '');
    });

    function refresh() {
      summary.textContent = chosen.size
        ? t('files.pickerSelected', { count: chosen.size }, `${chosen.size} selected`)
        : t('files.pickerNoSelection', null, 'Nothing selected');
      openBtn.disabled = chosen.size === 0;
      for (const [path, cell] of cells) cell.classList.toggle('is-selected', chosen.has(path));
    }

    function choose(entry) {
      if (!multiple) chosen.clear();
      if (chosen.has(entry.path)) chosen.delete(entry.path);
      else {
        if (entry.size > MAX_PICK_BYTES) {
          toast(t('files.pickerTooLarge', { name: entry.name }, `${entry.name} is too large to open`), {
            type: 'error',
          });
          return;
        }
        chosen.set(entry.path, entry);
      }
      refresh();
    }

    function render() {
      gridHost.textContent = '';
      cells.clear();
      for (const entry of entries) {
        const cell = entryCell(entry, {
          onPick: choose,
          onOpen: (picked) => {
            if (picked.kind === 'dir') {
              void load(picked.path);
              return;
            }
            if (!chosen.has(picked.path)) choose(picked);
            close([...chosen.values()]);
          },
        });
        cells.set(entry.path, cell);
        gridHost.appendChild(cell);
      }
      if (!entries.length) {
        gridHost.appendChild(emptyState({
          icon: 'ui/folder',
          title: t('files.pickerEmpty', null, 'No files here'),
        }));
      }
      refresh();
    }

    async function load(path) {
      try {
        const query = path ? `?path=${encodeURIComponent(path)}` : '';
        const response = await apiFetch(path ? `/api/files/list${query}` : '/api/files/home');
        const data = response?.data ?? {};
        current = data.path ?? '';
        entries = (data.entries ?? []).filter(
          (entry) => entry && entry.name && !entry.hidden && (entry.kind === 'dir' || matches(entry)),
        );
        pathText.textContent = `${data.home ?? '~'}${current ? `/${current}` : ''}`;
        upBtn.disabled = !current;
        chosen.clear();
        render();
      } catch (error) {
        toast(error?.message || t('files.pickerUnavailable', null, 'Could not read this folder'), {
          type: 'error',
        });
      }
    }

    dialog.open();
    void load('');
  });
}

/**
 * Pick files and hand back `File` objects (the shell's own callers: plugin
 * upload, wallpaper). `null` means the picker is unavailable — the caller keeps
 * its old `<input type=file>` path.
 */
export async function pickFilesFromShell({ accept = '', multiple = false } = {}) {
  const picked = await pickPaths({ accept, multiple });
  if (picked === null) return null;
  try {
    return await filesFromEntries(globalThis, picked);
  } catch (error) {
    toast(error?.message || t('files.pickerUnavailable', null, 'Could not read this folder'), {
      type: 'error',
    });
    return [];
  }
}
