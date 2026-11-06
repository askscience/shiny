import { apiFetch, getTraveler } from '../api.js';
import { toast, input, toggleRow, select, revealRegion, disposeMotion, controlCard } from '../../ui/index.js';
import { t } from '../i18n.js';
import { saveKnownUser, renderAvatarEl, readAvatarFile } from '../userProfiles.js';
import { getLockScreen, setLockScreen, LOCK_IDLE_MIN, LOCK_IDLE_MAX } from '../preferences.js';
import { el, field, heading } from './controls.js';

/** Build the account section with resources owned by its current mount. */
export function createAccountSection({ on, cleanups, resources }) {
  function buildAccount() {
    const traveler = getTraveler();

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

    // The profile saves as it changes, like every other preference here.
    async function save(body) {
      try {
        const res = await apiFetch('/api/travelers/me', { method: 'PUT', body: JSON.stringify(body) });
        if (res?.data) {
          localStorage.setItem('traveler', JSON.stringify(res.data));
          saveKnownUser(res.data);
          window.dispatchEvent(new CustomEvent('profile:change'));
        }
      } catch (e) {
        toast(e.message || t('settings.account.saveFailed', null, 'Could not save profile'), { type: 'error' });
      }
    }

    on(avatarInput, 'change', async () => {
      const file = avatarInput.files?.[0];
      if (!file) return;
      try {
        const avatar = await readAvatarFile(file);
        renderAvatarEl(avatarPreview, { name: nameInput.value || traveler?.name || '', avatar });
        await save({ avatar });
      } catch (e) {
        toast(e.message, { type: 'error' });
      }
      avatarInput.value = '';
    });

    on(nameInput, 'change', () => {
      const name = nameInput.value.trim();
      if (!name || name === (getTraveler()?.name || '')) return;
      void save({ name });
    });

    return [
      upload,
      field(t('settings.account.displayName', null, 'Display name'), nameInput),
      ...buildLock(),
    ];
  }

  /* ── Lock screen (js/lockScreen.js) ─────────────────────────── */

  function buildLock() {
    const lock = getLockScreen();

    // "Lock after": an amount and its unit — seconds, minutes, or never
    // (the lock screen stays there on demand; nothing locks by itself).
    const unitOf = (idle) => (!idle ? 'never' : (idle % 60 === 0 ? 'min' : 'sec'));
    let unit = unitOf(lock.idle);
    const amount = input({
      type: 'number',
      min: 1,
      step: 1,
      inputMode: 'numeric',
      label: t('settings.lock.amount', null, 'Amount'),
      className: 'settings-lock-amount',
    });
    const showAmount = (idle) => {
      amount.value = unit === 'min' ? String(idle / 60) : unit === 'sec' ? String(idle) : '';
      amount.max = String(unit === 'min' ? LOCK_IDLE_MAX / 60 : LOCK_IDLE_MAX);
      amount.min = String(unit === 'min' ? 1 : LOCK_IDLE_MIN);
      amount.hidden = unit === 'never';
    };
    const commit = (seconds) => {
      const idle = seconds <= 0 ? 0 : Math.round(Math.min(LOCK_IDLE_MAX, Math.max(LOCK_IDLE_MIN, seconds)));
      setLockScreen({ idle });
      showAmount(idle);
    };
    on(amount, 'change', () => {
      const n = Math.max(1, Math.round(Number(amount.value) || 0));
      commit(unit === 'min' ? n * 60 : n);
    });

    const unitSelect = select({
      options: [
        { value: 'sec', label: t('settings.lock.seconds', null, 'Seconds') },
        { value: 'min', label: t('settings.lock.minutes', null, 'Minutes') },
        { value: 'never', label: t('settings.lock.never', null, 'Never') },
      ],
      onChange: (value) => {
        const current = getLockScreen().idle;
        unit = value;
        if (value === 'never') commit(0);
        else if (value === 'min') commit(current ? Math.max(60, Math.round(current / 60) * 60) : 5 * 60);
        else commit(current || 30);
      },
    });
    unitSelect.select.value = unit;
    unitSelect.select.setAttribute('aria-label', t('settings.lock.unit', null, 'Unit'));
    showAmount(lock.idle);

    const after = el('div', 'settings-lock-after');
    after.append(amount, unitSelect);

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
