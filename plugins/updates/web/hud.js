/**
 * hud.js — the Updates top-bar chip.
 *
 * Loaded by core's `pluginHud.js` whenever the Updates plugin is **installed**
 * (not only while it is active), so the update badge stays visible even with
 * the window closed. The button is injected into `#hud-top` immediately before
 * the battery chip — the same cluster as sound, Wi-Fi and Bluetooth.
 *
 * Clicking it activates/focuses the Updates window. The chip polls
 * `/api/updates/status` and shows the pending count; a desktop notification is
 * raised on a 0 → N transition.
 */
import { notify, setIcon } from '../../ui/index.js';
import { apiFetch } from '../../js/api.js';
import { activatePlugin } from '../../js/tiles.js';

const PLUGIN = 'updates';
const POLL_MS = 5 * 60 * 1000;

let btn = null;
let badgeEl = null;
let timer = null;
let status = null;
let lastCount = null;

export function install() {
  if (btn && btn.isConnected) return;
  const top = document.getElementById('hud-top');
  if (!top) return;
  const battery = document.getElementById('hud-battery');

  // Styled like the other host status chips (sound / Wi-Fi / Bluetooth), not
  // like the round Settings/Plugins/Power buttons — see `.hud-updates` in
  // web/css/app.css.
  btn = document.createElement('button');
  btn.type = 'button';
  btn.id = 'hud-updates';
  btn.className = 'hud-updates';
  btn.title = 'Updates';
  btn.setAttribute('aria-label', 'Updates');
  btn.setAttribute('aria-haspopup', 'dialog');
  btn.style.position = 'relative';

  const ic = document.createElement('span');
  ic.className = 'hud-updates-icon';
  ic.setAttribute('aria-hidden', 'true');
  btn.appendChild(ic);

  badgeEl = document.createElement('span');
  badgeEl.className = 'hud-updates-badge';
  badgeEl.setAttribute('aria-hidden', 'true');
  badgeEl.style.cssText = [
    'position:absolute', 'top:-3px', 'right:-3px', 'min-width:15px', 'height:15px',
    'box-sizing:border-box', 'padding:0 4px', 'border-radius:999px',
    'background:var(--accent)', 'color:var(--bg,#fff)', 'font-size:9px',
    'line-height:15px', 'font-weight:700', 'text-align:center', 'display:none',
    'pointer-events:none',
  ].join(';');
  btn.appendChild(badgeEl);

  setIcon(ic, 'ui/download', { size: 20 });
  btn.addEventListener('click', () => void activatePlugin(PLUGIN));

  if (battery && battery.parentElement === top) top.insertBefore(btn, battery);
  else top.appendChild(btn);

  void refresh();
  if (!timer) timer = setInterval(() => void refresh(), POLL_MS);
}

export function uninstall() {
  btn?.remove();
  btn = null;
  badgeEl = null;
  status = null;
  lastCount = null;
  if (timer) {
    clearInterval(timer);
    timer = null;
  }
}

async function refresh() {
  try {
    const res = await apiFetch('/api/updates/status');
    if (res?.data) status = res.data;
  } catch (_) {
    // Server restarting or offline — keep the last known chip state.
  }
  update();
}

function update() {
  if (!btn) return;
  const count = status?.count ?? 0;
  const error = status?.error;

  // The chip is a notification, not a permanent button: it only exists on the
  // bar while a (background) check has found updates.
  btn.style.display = count > 0 ? '' : 'none';

  if (count > 0) {
    badgeEl.style.display = 'block';
    badgeEl.textContent = count > 99 ? '99+' : String(count);
    btn.classList.add('is-active');
    btn.title = `${count} update${count === 1 ? '' : 's'} available`;
  } else {
    badgeEl.style.display = 'none';
    btn.classList.remove('is-active');
    btn.title = error ? 'Updates unavailable' : 'System up to date';
  }
  btn.setAttribute('aria-label', btn.title);

  if (lastCount === 0 && count > 0) {
    notify({
      app: 'Updates',
      title: 'Updates available',
      body: `${count} update${count === 1 ? '' : 's'} ready to install`,
      icon: 'ui/download',
      urgency: 'normal',
    });
  }
  lastCount = count;
}

export default { install, uninstall };
