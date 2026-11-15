/**
 * hud.js — Files' session-wide watch on removable drives.
 *
 * Loaded by core's `pluginHud.js` while Files is installed, window open or
 * not. It adds no chip: it polls `/api/files/volumes`, shares each new list
 * with the window (`files:volumes`), and raises a notification when a drive
 * becomes available (the server mounts it on insertion when automount is on),
 * with an Open button that shows it in Files.
 */
import { notify } from '../../ui/index.js';
import { apiFetch } from '../../js/api.js';
import { revealInFiles } from '../../js/files.js';
import { t } from '../../js/i18n.js';

const POLL_MS = 4000;
const OPEN_PREFIX = 'files:open-volume:';

let timer = null;
let known = null; // ids of mounted drives at the last poll (null: first poll)
let lastJson = '';

async function poll() {
  let data = null;
  try {
    const res = await apiFetch('/api/files/volumes', { authRedirect: false });
    data = res?.data || null;
  } catch (_) {
    return;
  }
  if (!data) return;
  const json = JSON.stringify(data);
  if (json !== lastJson) {
    lastJson = json;
    window.dispatchEvent(new CustomEvent('files:volumes', { detail: data }));
  }
  const volumes = Array.isArray(data.volumes) ? data.volumes : [];
  const mountedNow = new Set(volumes.filter((v) => v.mounted).map((v) => v.id));
  if (known) {
    for (const vol of volumes) {
      if (!vol.mounted || known.has(vol.id)) continue;
      notify({
        app: 'Files',
        title: t('plugin.files.volume.connected', null, 'Drive connected'),
        body: vol.label,
        icon: 'ui/usb-drive',
        urgency: 'low',
        actions: [{ label: t('plugin.files.volume.open', null, 'Open'), action: `${OPEN_PREFIX}${vol.id}` }],
      });
    }
  }
  known = mountedNow;
}

function onAction(e) {
  const action = e.detail?.action;
  if (typeof action !== 'string' || !action.startsWith(OPEN_PREFIX)) return;
  revealInFiles(`@volumes/${action.slice(OPEN_PREFIX.length)}`);
}

export function install() {
  if (timer) return;
  window.addEventListener('app:notification-action', onAction);
  void poll();
  timer = setInterval(() => { if (!document.hidden) void poll(); }, POLL_MS);
}

export function uninstall() {
  clearInterval(timer);
  timer = null;
  known = null;
  lastJson = '';
  window.removeEventListener('app:notification-action', onAction);
}

export default { install, uninstall };
