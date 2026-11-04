import { apiFetch, getTraveler } from '../api.js';
import { toast, button, input, toggleRow, select, revealRegion, disposeMotion, controlCard } from '../../ui/index.js';
import { t, tp } from '../i18n.js';
import { saveKnownUser, renderAvatarEl, readAvatarFile } from '../userProfiles.js';
import { getLockScreen, setLockScreen, LOCK_AFTER_CHOICES } from '../preferences.js';
import { el, field, heading } from './controls.js';

/** Build the account section with resources owned by its current mount. */
export function createAccountSection({ on, cleanups, resources }) {
  function buildAccount() {
    const traveler = getTraveler();
    let pendingAvatar;

    const avatarPreview = el('div', 'profile-avatar profile-avatar--large');
    renderAvatarEl(avatarPreview, {
      name: traveler?.name || traveler?.username || '',
      avatar: traveler?.avatar || null,
    });

    const avatarInput = input({ type: 'file' });
    avatarInput.accept = 'image/*';
    avatarInput.hidden = true;

    const upload = el('label', 'profile-avatar-upload');
    upload.append(avatarInput, avatarPreview, el('span', 'profile-avatar-label', t('settings.account.changePhoto', null, 'Change photo')));

    const nameInput = input({
      value: traveler?.name || '',
      maxlength: 64,
      autocomplete: 'name',
    });

    on(avatarInput, 'change', async () => {
      const file = avatarInput.files?.[0];
      if (!file) return;
      try {
        pendingAvatar = await readAvatarFile(file);
        renderAvatarEl(avatarPreview, { name: nameInput.value || traveler?.name || '', avatar: pendingAvatar });
      } catch (e) {
        toast(e.message, { type: 'error' });
        avatarInput.value = '';
      }
    });

    const saveBtn = button({
      label: t('settings.account.save', null, 'Save profile'),
      variant: 'primary',
      size: 'sm',
      onClick: async () => {
        const name = nameInput.value.trim();
        const body = {};
        if (name && name !== traveler?.name) body.name = name;
        if (pendingAvatar !== undefined) body.avatar = pendingAvatar;
        if (!Object.keys(body).length) {
          toast(t('settings.account.nothingToSave', null, 'Nothing to save'), { type: 'info' });
          return;
        }
        try {
          const res = await apiFetch('/api/travelers/me', { method: 'PUT', body: JSON.stringify(body) });
          if (res?.data) {
            localStorage.setItem('traveler', JSON.stringify(res.data));
            saveKnownUser(res.data);
            pendingAvatar = undefined;
            window.dispatchEvent(new CustomEvent('profile:change'));
            toast(t('settings.account.saved', null, 'Profile saved'), { type: 'info' });
          }
        } catch (e) {
          toast(e.message || t('settings.account.saveFailed', null, 'Could not save profile'), { type: 'error' });
        }
      },
    });

    const row = el('div', 'settings-inline-actions');
    row.appendChild(saveBtn);

    return [
      upload,
      field(t('settings.account.displayName', null, 'Display name'), nameInput),
      row,
      ...buildLock(),
    ];
  }

  /* ── Lock screen (js/lockScreen.js) ─────────────────────────── */

  function buildLock() {
    const lock = getLockScreen();

    const after = select({
      options: LOCK_AFTER_CHOICES.map((m) => ({
        value: String(m),
        label: tp('settings.lock.minutes', m, { count: m }, `${m} min`),
      })),
      onChange: (value) => setLockScreen({ after: Number(value) }),
    });
    after.select.value = String(lock.after);

    const password = toggleRow({
      label: t('settings.lock.password', null, 'Ask for the password'),
      checked: lock.password,
      onChange: (on) => setLockScreen({ password: on }),
    });

    // The details only matter while the lock screen is on.
    const details = el('div', 'settings-lock-details');
    cleanups.push(() => disposeMotion(details));
    details.append(password, field(t('settings.lock.after', null, 'Lock after'), after));

    const enabled = toggleRow({
      label: t('settings.lock.enabled', null, 'Lock screen'),
      checked: lock.enabled,
      onChange: (on) => {
        setLockScreen({ enabled: on });
        revealRegion(details, on);
      },
    });
    revealRegion(details, lock.enabled, { animate: false });

    const card = controlCard({ wide: true });
    card.classList.add('settings-field');
    card.append(enabled, details);

    return [
      heading(t('settings.lock.title', null, 'Lock screen')),
      card,
    ];
  }

  /* ── Assistant ──────────────────────────────────────────────── */


  return buildAccount();
}
