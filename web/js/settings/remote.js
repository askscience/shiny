import { apiFetch } from '../api.js';
import { toast, button, toggleRow } from '../../ui/index.js';
import { t, tp } from '../i18n.js';
import { getRemoteAllowTerminal, setRemoteAllowTerminal } from '../preferences.js';
import { copyText } from '../clipboard.js';
import { el, heading } from './controls.js';

/** Build the remote section with resources owned by its current mount. */
export function createRemoteSection({ on, cleanups, resources }) {
  function formatBytes(n) {
    if (!n) return '0 B';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let i = 0;
    let v = n;
    while (v >= 1024 && i < units.length - 1) { v /= 1024; i += 1; }
    return `${v < 10 && i > 0 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
  }

  function buildRemote() {
    const state = { enabled: false, endpointId: null, ticket: null, connections: 0, bytes: 0, paired: 0, pairing: false, tailscale: null };
    let revealed = false;

    const serverToggle = toggleRow({
      label: t('settings.remote.server', null, 'Server'),
      hint: t('settings.remote.serverHint', null, 'Let your other devices reach this desktop over Iroh — end-to-end encrypted, no port forwarding. The link changes every time you start or stop.'),
      checked: false,
      onChange: setEnabled,
    });

    const terminalToggle = toggleRow({
      label: t('settings.remote.allowTerminal', null, 'Allow Terminal from remote clients'),
      hint: t('settings.remote.allowTerminalHint', null, 'The Terminal is a real shell on this machine. Off by default; enable it only for devices you trust.'),
      checked: getRemoteAllowTerminal(),
      onChange: (on) => { setRemoteAllowTerminal(on); },
    });

    const statusLine = el('p', 'settings-hint', t('settings.remote.reading', null, 'Reading…'));
    const ticketEl = el('code', 'remote-ticket');
    const qrEl = el('img', 'remote-qr');
    qrEl.alt = t('settings.remote.qrAlt', null, 'Connection QR code');

    const revealBtn = button({
      label: t('settings.remote.revealLink', null, 'Reveal link'),
      variant: 'ghost',
      onClick: () => { revealed = !revealed; render(); },
    });
    const copyBtn = button({
      label: t('common.copy'),
      variant: 'ghost',
      onClick: async () => {
        if (!state.ticket) return;
        // Copies go through the central service so they land in the history.
        const ok = await copyText(state.ticket, { source: 'settings' });
        toast(ok ? t('settings.remote.linkCopied', null, 'Link copied') : t('settings.remote.copyFailed', null, 'Could not copy the link'), ok ? undefined : { type: 'error' });
      },
    });
    const rotateBtn = button({ label: t('settings.remote.rotateKey', null, 'Rotate key'), variant: 'danger', onClick: rotate });
    const pairBtn = button({ label: t('settings.remote.pairDevice', null, 'Pair a new device'), variant: 'ghost', onClick: pair });
    const unpairBtn = button({ label: t('settings.remote.forgetDevices', null, 'Forget devices'), variant: 'quiet', onClick: unpair });

    const actions = el('div', 'settings-inline-actions');
    actions.append(revealBtn, copyBtn, rotateBtn, pairBtn, unpairBtn);

    const linkBox = el('div', 'remote-link');
    linkBox.append(ticketEl, qrEl);
    const linkHint = el('p', 'settings-hint', t('settings.remote.linkHint', null, 'On another device: peakd --iroh <link> opens the app in a window. To use a plain browser instead, turn on the public URL below.'));

    // ── Tailscale Funnel: a normal https:// URL for any browser ──────────
    const tsToggle = toggleRow({
      label: t('settings.remote.publicUrl', null, 'Public URL (Tailscale Funnel)'),
      hint: t('settings.remote.publicUrlHint', null, 'Serve the app at an https://…ts.net URL through Tailscale — open it in any browser, on any device, no app or port forwarding. Free; needs Tailscale installed and signed in.'),
      checked: false,
      onChange: setTailscale,
    });
    const tsMissing = el('p', 'settings-hint', t('settings.remote.tailscaleMissing', null, 'Tailscale is not installed on this machine.'));
    const tsStatus = el('p', 'settings-hint', '');
    const tsUrlEl = el('code', 'remote-ticket');
    const tsQrEl = el('img', 'remote-qr');
    tsQrEl.alt = t('settings.remote.tailscaleQrAlt', null, 'Tailscale URL QR code');
    const tsUrlBox = el('div', 'remote-link');
    tsUrlBox.append(tsUrlEl, tsQrEl);
    const tsCopyBtn = button({ label: t('settings.remote.copyUrl', null, 'Copy URL'), variant: 'ghost', onClick: copyTailscale });
    const tsActions = el('div', 'settings-inline-actions');
    tsActions.append(tsCopyBtn);

    function apply(s) {
      if (!s) return;
      state.enabled = !!s.enabled;
      state.endpointId = s.endpoint_id || null;
      state.ticket = s.ticket || null;
      state.connections = s.connections || 0;
      state.bytes = s.bytes || 0;
      state.paired = s.paired || 0;
      state.pairing = !!s.pairing;
      state.tailscale = s.tailscale || null;
      render();
    }

    function render() {
      serverToggle.toggle.setChecked(state.enabled, { silent: true });
      renderTailscale();
      if (!state.enabled) {
        statusLine.textContent = t('settings.remote.offLocalOnly', null, 'Off. This desktop is reachable only on this machine.');
        linkBox.classList.add('hidden');
        actions.classList.add('hidden');
        return;
      }
      const id = state.endpointId ? `${state.endpointId.slice(0, 12)}…` : '';
      const connections = tp('settings.remote.connections', state.connections, null, `${state.connections} connection${state.connections === 1 ? '' : 's'}`);
      let line = t('settings.remote.statusOn', { id, connections, bytes: formatBytes(state.bytes) }, `On — ${id} · ${connections} · ${formatBytes(state.bytes)}`);
      if (state.paired > 0) line += ` · ${tp('settings.remote.paired', state.paired, null, `${state.paired} paired device${state.paired === 1 ? '' : 's'}`)}`;
      if (state.pairing) line += ` · ${t('settings.remote.pairing', null, 'pairing for the next device…')}`;
      statusLine.textContent = line;
      linkBox.classList.remove('hidden');
      actions.classList.remove('hidden');
      revealBtn.textContent = revealed ? t('settings.remote.hideLink', null, 'Hide link') : t('settings.remote.revealLink', null, 'Reveal link');
      if (revealed) {
        ticketEl.textContent = state.ticket || '';
        qrEl.src = `/api/remote/qr?t=${Date.now()}`;
        qrEl.classList.remove('hidden');
      } else {
        ticketEl.textContent = '••••••••••••••••••••••••••••••';
        qrEl.removeAttribute('src');
        qrEl.classList.add('hidden');
      }
    }

    function renderTailscale() {
      const ts = state.tailscale;
      const installed = !!(ts && ts.installed);
      tsMissing.classList.toggle('hidden', installed);
      tsToggle.classList.toggle('hidden', !installed);
      if (!installed) {
        tsStatus.textContent = ts && ts.error ? t('settings.remote.tailscaleError', { error: ts.error }, `Tailscale: ${ts.error}`) : '';
        tsUrlBox.classList.add('hidden');
        tsActions.classList.add('hidden');
        return;
      }
      const on = !!ts.funnel;
      tsToggle.toggle.setChecked(on, { silent: true });
      let line;
      if (!ts.up) line = t('settings.remote.tailscaleDown', null, 'Installed, but not connected — run `tailscale up` on this machine.');
      else if (on) line = t('settings.remote.tailscaleOn', { host: ts.hostname || t('settings.remote.thisDevice', null, 'this device') }, `On — ${ts.hostname || 'this device'}`);
      else line = t('settings.remote.tailscaleOff', null, 'Off. The app is reachable only locally and over Iroh.');
      if (ts.error) line += t('settings.remote.errorNote', { error: ts.error }, ` (${ts.error})`);
      tsStatus.textContent = line;
      if (on && ts.url) {
        tsUrlEl.textContent = ts.url;
        tsUrlBox.classList.remove('hidden');
        tsActions.classList.remove('hidden');
        tsQrEl.src = `/api/remote/tailscale/qr?t=${Date.now()}`;
      } else {
        tsUrlBox.classList.add('hidden');
        tsActions.classList.add('hidden');
      }
    }

    async function setTailscale(on) {
      try {
        await apiFetch(`/api/remote/tailscale/${on ? 'enable' : 'disable'}`, {
          method: 'POST',
          authRedirect: false,
        });
        toast(on ? t('settings.remote.funnelOn', null, 'Tailscale Funnel on') : t('settings.remote.funnelOff', null, 'Tailscale Funnel off'));
        await refresh();
      } catch (e) {
        tsToggle.toggle.setChecked(!on, { silent: true });
        toast(e.message || t('settings.remote.funnelFailed', null, 'Could not change Tailscale Funnel'), { type: 'error' });
      }
    }

    async function copyTailscale() {
      const url = state.tailscale && state.tailscale.url;
      if (!url) return;
      const ok = await copyText(url, { source: 'settings' });
      toast(ok ? t('settings.remote.urlCopied', null, 'URL copied') : t('settings.remote.urlCopyFailed', null, 'Could not copy the URL'), ok ? undefined : { type: 'error' });
    }

    async function refresh() {
      try { apply(await apiFetch('/api/remote/status', { authRedirect: false })); } catch (_) {}
    }

    async function setEnabled(on) {
      try {
        const res = await apiFetch('/api/remote/enable', {
          method: 'POST',
          authRedirect: false,
          body: JSON.stringify({ enabled: on }),
        });
        if (on) {
          apply(res);
          // In the kiosk, hand the screen to the server-mode window (the shell
          // exits with a mode-switch code). A plain browser — or a remote client,
          // which cannot enable anyway — has no bridge and just shows the link.
          window.ipc?.postMessage?.('peakd:server-mode');
        } else {
          apply({ enabled: false });
        }
        toast(on ? t('settings.remote.serverOn', null, 'Server mode on') : t('settings.remote.serverOff', null, 'Server mode off'));
      } catch (e) {
        serverToggle.toggle.setChecked(!on, { silent: true });
        toast(e.message || t('settings.remote.serverFailed', null, 'Could not change server mode'), { type: 'error' });
      }
    }

    async function rotate() {
      try {
        apply(await apiFetch('/api/remote/rotate', { method: 'POST', authRedirect: false }));
        revealed = false;
        render();
        toast(t('settings.remote.newLink', null, 'New link generated'));
      } catch (e) {
        toast(e.message || t('settings.remote.rotateFailed', null, 'Could not rotate the link'), { type: 'error' });
      }
    }

    async function pair() {
      try {
        const res = await apiFetch('/api/remote/pair', { method: 'POST', authRedirect: false });
        toast(t('settings.remote.pairingOpen', { seconds: res?.seconds || 120 }, `Pairing open for ${res?.seconds || 120}s — connect from the new device now`));
        refresh();
      } catch (e) {
        toast(e.message || t('settings.remote.pairFailed', null, 'Could not start pairing'), { type: 'error' });
      }
    }

    async function unpair() {
      try {
        const res = await apiFetch('/api/remote/unpair', { method: 'POST', authRedirect: false });
        toast(tp('settings.remote.forgotDevices', res?.removed ?? 0, null, `Forgot ${res?.removed ?? 0} paired device(s)`));
        refresh();
      } catch (e) {
        toast(e.message || t('settings.remote.forgetFailed', null, 'Could not forget devices'), { type: 'error' });
      }
    }

    // Poll only while the Remote panel is visible; stop once it is detached.
    const timer = resources.interval(() => {
      if (!statusLine.isConnected) { clearInterval(timer); return; }
      const panel = statusLine.closest('.settings-panel');
      if (panel && panel.classList.contains('is-active')) refresh();
    }, 3000);
    refresh();

    return [
      serverToggle,
      statusLine,
      linkBox,
      linkHint,
      actions,
      heading(t('settings.remote.publicUrlHeading', null, 'Public URL (Tailscale)')),
      tsMissing,
      tsToggle,
      tsStatus,
      tsUrlBox,
      tsActions,
      heading(t('settings.remote.remoteControl', null, 'Remote control')),
      terminalToggle,
    ];
  }

  /* ── Browser ─────────────────────────────────────────────────── */

  /**
   * The Browser window's settings, mirrored here so the search engine can be
   * chosen from Settings as well as from the window's own ⋯ menu. The values
   * live server-side, per user; this section reads them and writes them straight
   * back, like every other preference here.
   */

  return buildRemote();
}
