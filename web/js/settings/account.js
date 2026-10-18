import { apiFetch, getTraveler } from '../api.js';
import { toast, button, input } from '../../ui/index.js';
import { t } from '../i18n.js';
import { saveKnownUser, renderAvatarEl, readAvatarFile } from '../userProfiles.js';
import { el, field } from './controls.js';

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
    ];
  }

  /* ── Assistant ──────────────────────────────────────────────── */


  return buildAccount();
}
