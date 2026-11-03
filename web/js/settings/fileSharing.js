import { apiFetch } from '../api.js';
import { toast, button, input, select, toggleRow } from '../../ui/index.js';
import { t } from '../i18n.js';
import { copyText } from '../clipboard.js';
import { el, field, heading } from './controls.js';

/**
 * Settings → Remote → "File sharing": host setup for the Files plugin's public
 * share links (`plugins/files/src/share`). Shiny runs a Gokapi server on this
 * machine; this block turns it on or off and chooses how recipients reach it:
 * Tailscale Funnel on :8443 (Shiny's own Funnel keeps :443) or a public URL
 * the user already forwards (reverse proxy / own domain).
 *
 * The package bundles Gokapi; a development checkout without it gets a
 * "Download Gokapi" button instead. The whole block stays hidden when the
 * Files plugin is not installed (its status route answers 404).
 */
export function createFileSharingBlock({ on, resources }) {
  const tr = (key, fallback, vars = null) => t(`settings.remote.share.${key}`, vars, fallback);
  const state = { status: null, tailscale: null, busy: false };

  const head = heading(tr('heading', 'File sharing'));
  const wrap = el('div', 'settings-file-sharing');
  const setVisible = (visible) => { head.hidden = !visible; wrap.hidden = !visible; };
  setVisible(false);

  const statusLine = el('p', 'settings-hint', tr('reading', 'Reading…'));

  const enableToggle = toggleRow({
    label: tr('enable', 'Public share links'),
    hint: tr('enableHint', 'Let “Share link…” in Files create download links anyone can open. Shiny runs a small Gokapi server on this machine for them.'),
    checked: false,
    onChange: (enabled) => void (enabled ? applySetup() : disable()),
  });

  const modeSel = select({
    options: [
      { value: 'funnel', label: tr('modeFunnel', 'Tailscale Funnel (https://…ts.net:8443)') },
      { value: 'custom', label: tr('modeCustom', 'My own public URL') },
    ],
    onChange: () => syncMode(),
  });
  const modeHint = el('p', 'settings-hint', '');
  const urlInput = input({ type: 'url', placeholder: 'https://share.example.com/', autocomplete: 'off' });
  const urlField = field(tr('url', 'Public URL'), urlInput, {
    hint: tr('urlHint', 'Your reverse proxy must forward this URL to the local share gate (its address appears here once sharing is on).'),
  });
  const applyBtn = button({ label: tr('apply', 'Apply'), variant: 'ghost', onClick: () => void applySetup() });

  const urlEl = el('code', 'remote-ticket');
  const copyBtn = button({ label: tr('copyUrl', 'Copy URL'), variant: 'ghost', onClick: copyUrl });
  const urlRow = el('div', 'settings-inline-actions');
  urlRow.append(urlEl, copyBtn);

  const downloadHint = el('p', 'settings-hint', tr('notBundled', 'Gokapi is not bundled with this build of Shiny.'));
  const downloadBtn = button({ label: tr('download', 'Download Gokapi'), variant: 'ghost', onClick: () => void download() });
  const downloadRow = el('div', 'settings-inline-actions');
  downloadRow.append(downloadBtn);

  const actions = el('div', 'settings-inline-actions');
  actions.append(applyBtn);

  wrap.append(
    enableToggle,
    statusLine,
    field(tr('mode', 'How recipients reach the links'), modeSel, { id: modeSel.select.id }),
    modeHint,
    urlField,
    actions,
    urlRow,
    downloadHint,
    downloadRow,
  );

  function mode() { return modeSel.select.value; }

  function syncMode() {
    const ts = state.tailscale || {};
    urlField.classList.toggle('hidden', mode() !== 'custom');
    if (mode() === 'funnel') {
      if (!ts.installed) modeHint.textContent = tr('tsMissing', 'Tailscale is not installed. Install it from tailscale.com/download, then run “sudo tailscale up --operator=$USER” once.');
      else if (!ts.up) modeHint.textContent = tr('tsDown', 'Tailscale is installed but not connected. Run “sudo tailscale up --operator=$USER” once.');
      else modeHint.textContent = tr('tsReady', 'Links will look like https://{host}:8443/… — reachable from anywhere, no router setup.', { host: ts.hostname || 'this-device.ts.net' });
    } else {
      const port = state.status?.forward_port;
      modeHint.textContent = port
        ? tr('forwardTo', 'Forward your URL to http://127.0.0.1:{port} — only download pages are served there; Gokapi’s admin and API stay private.', { port })
        : '';
    }
    const s = state.status || {};
    // "Apply" only matters when sharing is on and the choice differs from what runs.
    const changed = s.enabled && (mode() !== s.mode || (mode() === 'custom' && normalized(urlInput.value) !== (s.public_url || '')));
    actions.classList.toggle('hidden', !changed);
  }

  function normalized(url) {
    const u = url.trim();
    return u && !u.endsWith('/') ? `${u}/` : u;
  }

  function render() {
    const s = state.status;
    if (!s) return;
    setVisible(true);
    const installed = !!s.installed;
    downloadHint.classList.toggle('hidden', installed);
    downloadRow.classList.toggle('hidden', installed);
    enableToggle.toggle.setChecked(!!s.enabled, { silent: true });
    enableToggle.toggle.disabled = !installed || state.busy;

    const install = s.install || {};
    let line;
    if (!installed) {
      line = install.state === 'running'
        ? tr('downloading', 'Downloading Gokapi… {step}', { step: install.step || '' })
        : install.state === 'failed'
          ? tr('downloadFailed', 'Download failed: {error}', { error: install.error || '' })
          : tr('missing', 'Gokapi is not available.');
    } else if (!s.enabled) {
      line = tr('off', 'Off. “Share link…” in Files asks you to turn this on first.');
    } else if (s.running) {
      line = tr('on', 'On — links are served from {url}', { url: s.public_url || '' });
    } else {
      line = tr('stopped', 'On, but the sharing server is not running.');
    }
    if (s.error) line += ` (${s.error})`;
    statusLine.textContent = line;

    // Keep what the user is editing; otherwise mirror the running setup.
    if (!wrap.contains(document.activeElement)) {
      const ts = state.tailscale || {};
      modeSel.select.value = s.mode || (ts.installed ? 'funnel' : 'custom');
      if (s.mode === 'custom') urlInput.value = s.public_url || '';
    }
    urlEl.textContent = s.public_url || '';
    urlRow.classList.toggle('hidden', !(s.enabled && s.public_url));
    syncMode();
  }

  async function refresh() {
    try {
      const [share, remote] = await Promise.all([
        apiFetch('/api/files/share/status', { authRedirect: false }),
        apiFetch('/api/remote/status', { authRedirect: false }).catch(() => null),
      ]);
      state.status = share?.data || null;
      state.tailscale = remote?.tailscale || null;
      render();
    } catch (e) {
      // 404: the Files plugin is not installed — keep the block hidden.
      if (e?.status === 404) setVisible(false);
    }
  }

  async function applySetup() {
    if (state.busy) return;
    state.busy = true;
    render();
    statusLine.textContent = tr('starting', 'Starting the sharing server…');
    try {
      const res = await apiFetch('/api/files/share/setup', {
        method: 'POST',
        authRedirect: false,
        body: JSON.stringify({ mode: mode(), url: urlInput.value.trim() }),
      });
      state.status = res?.data || state.status;
      toast(tr('enabled', 'Link sharing is on'));
    } catch (e) {
      toast(e.message || tr('enableFailed', 'Could not turn on link sharing'), { type: 'error' });
    } finally {
      state.busy = false;
      await refresh();
    }
  }

  async function disable() {
    if (state.busy) return;
    state.busy = true;
    try {
      const res = await apiFetch('/api/files/share/disable', { method: 'POST', authRedirect: false });
      state.status = res?.data || state.status;
      toast(tr('disabled', 'Link sharing is off'));
    } catch (e) {
      toast(e.message || tr('disableFailed', 'Could not turn off link sharing'), { type: 'error' });
    } finally {
      state.busy = false;
      await refresh();
    }
  }

  async function download() {
    downloadBtn.disabled = true;
    try {
      await apiFetch('/api/files/share/install', { method: 'POST', authRedirect: false });
      await refresh();
    } catch (e) {
      toast(e.message || tr('downloadFailedToast', 'Could not download Gokapi'), { type: 'error' });
    } finally {
      downloadBtn.disabled = false;
    }
  }

  async function copyUrl() {
    const url = state.status?.public_url;
    if (!url) return;
    const ok = await copyText(url, { source: 'settings' });
    toast(ok ? tr('urlCopied', 'URL copied') : tr('urlCopyFailed', 'Could not copy the URL'), ok ? undefined : { type: 'error' });
  }

  on(urlInput, 'input', syncMode);

  // Poll while the Remote panel is visible (download progress, server state).
  const timer = resources.interval(() => {
    if (!wrap.isConnected) { clearInterval(timer); return; }
    const panel = wrap.closest('.settings-panel');
    if (panel && panel.classList.contains('is-active') && !state.busy) refresh();
  }, 3000);
  refresh();

  return [head, wrap];
}
