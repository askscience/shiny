import { apiFetch, getVoiceLang, setVoiceLang } from './api.js';
import { changeLanguage } from './voice.js';
import { refreshActiveTrip } from './gps.js';
import { loadActiveRoute } from './map.js';

const panel = document.getElementById('settings-panel');
const select = document.getElementById('lang-select');
const tripsList = document.getElementById('trips-list');
const openBtn = document.getElementById('settings-btn');
const closeBtn = document.getElementById('settings-close');

function statusLabel(status) {
  if (status === 'active') return 'Active';
  if (status === 'planned') return 'Planned';
  if (status === 'completed') return 'Done';
  return status;
}

async function loadTripsList() {
  if (!tripsList) return;
  tripsList.innerHTML = '<li class="trips-empty">Loading…</li>';
  try {
    const res = await apiFetch('/api/trips');
    const trips = res.data || [];
    if (!trips.length) {
      tripsList.innerHTML = '<li class="trips-empty">No trips yet</li>';
      return;
    }
    tripsList.innerHTML = '';
    trips.forEach((trip) => {
      const li = document.createElement('li');
      li.className = `trip-item trip-${trip.status}`;
      li.innerHTML = `
        <span class="trip-item-name">${trip.name}</span>
        <span class="trip-item-status">${statusLabel(trip.status)}</span>
      `;
      if (trip.status === 'planned') {
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'trip-start-btn';
        btn.textContent = 'Start';
        btn.addEventListener('click', async (e) => {
          e.stopPropagation();
          btn.disabled = true;
          try {
            await apiFetch(`/api/trips/${trip.id}/start`, { method: 'POST' });
            const active = await refreshActiveTrip();
            if (active?.id) await loadActiveRoute(active.id);
            window.dispatchEvent(new CustomEvent('trips:changed'));
            await loadTripsList();
          } catch (err) {
            btn.disabled = false;
            window.dispatchEvent(new CustomEvent('app:toast', {
              detail: { message: err.message || 'Could not start trip', type: 'error' },
            }));
          }
        });
        li.appendChild(btn);
      }
      tripsList.appendChild(li);
    });
  } catch (_) {
    tripsList.innerHTML = '<li class="trips-empty">Could not load trips</li>';
  }
}

export async function initSettings() {
  try {
    const res = await apiFetch('/api/voice/languages');
    select.innerHTML = '';
    res.data.forEach((lang) => {
      const opt = document.createElement('option');
      opt.value = lang.code;
      opt.textContent = `${lang.code.toUpperCase()}${lang.vosk_available ? '' : ' (TTS only)'}`;
      select.appendChild(opt);
    });
    select.value = getVoiceLang();
  } catch (_) {
    select.innerHTML = '<option value="en">EN</option>';
  }

  openBtn.addEventListener('click', () => {
    panel.classList.remove('hidden');
    loadTripsList();
  });
  closeBtn.addEventListener('click', async () => {
    const lang = select.value;
    if (lang !== getVoiceLang()) {
      setVoiceLang(lang);
      await changeLanguage(lang);
    }
    panel.classList.add('hidden');
  });

  window.addEventListener('trips:changed', loadTripsList);
}
