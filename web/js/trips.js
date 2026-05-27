import { apiFetch } from './api.js';
import { refreshActiveTrip } from './gps.js';
import { loadActiveRoute } from './map.js';

const panel = document.getElementById('trips-panel');
const tripsList = document.getElementById('trips-list');
const openBtn = document.getElementById('trips-btn');
const closeBtn = document.getElementById('trips-close');

function statusLabel(status) {
  if (status === 'active') return 'Active';
  if (status === 'planned') return 'Planned';
  if (status === 'completed') return 'Done';
  return status;
}

export async function loadTripsList() {
  if (!tripsList) return;
  tripsList.innerHTML = '<li class="trips-empty">Loading…</li>';
  try {
    const res = await apiFetch('/api/trips');
    const trips = res.data || [];
    if (!trips.length) {
      tripsList.innerHTML = '<li class="trips-empty">No trips yet — ask the AI to plan one</li>';
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

export function initTrips() {
  openBtn?.addEventListener('click', () => {
    panel?.classList.remove('hidden');
    loadTripsList();
  });
  closeBtn?.addEventListener('click', () => {
    panel?.classList.add('hidden');
  });
  panel?.addEventListener('click', (e) => {
    if (e.target === panel) panel.classList.add('hidden');
  });
  window.addEventListener('trips:changed', loadTripsList);
}
