const placements = new WeakMap();

/** Keep an open popup within the visual viewport until it is hidden or removed. */
export function placePopover(popup, anchor, options = {}) {
  placements.get(popup)?.();
  position(popup, anchor, options);
  if (typeof MutationObserver === 'undefined' || !popup.isConnected) return;
  const update = event => {
    if (!popup.isConnected || anchor.isConnected === false || popup.closest('[hidden],.hidden')) { dispose(); return; }
    if (event && popup.contains(event.target)) return;
    position(popup, anchor, options);
  };
  const observer = new MutationObserver(() => {
    if (!popup.isConnected || anchor.isConnected === false || popup.closest('[hidden],.hidden')) dispose();
  });
  const dispose = () => {
    observer.disconnect();
    window.removeEventListener('resize', update);
    document.removeEventListener('scroll', update, true);
    window.visualViewport?.removeEventListener('resize', update);
    window.visualViewport?.removeEventListener('scroll', update);
    placements.delete(popup);
  };
  placements.set(popup, dispose);
  observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['class', 'hidden'] });
  window.addEventListener('resize', update);
  document.addEventListener('scroll', update, true);
  window.visualViewport?.addEventListener('resize', update);
  window.visualViewport?.addEventListener('scroll', update);
}

function position(popup, anchor, { gap = 6, margin = 8 } = {}) {
  const viewport = window.visualViewport;
  const left = (viewport?.offsetLeft || 0) + margin;
  const top = (viewport?.offsetTop || 0) + margin;
  const width = Math.max(0, (viewport?.width || window.innerWidth) - margin * 2);
  const height = Math.max(0, (viewport?.height || window.innerHeight) - margin * 2);
  const rect = anchor.getBoundingClientRect();
  popup.style.boxSizing = 'border-box';
  popup.style.maxWidth = `${width}px`;
  popup.style.minWidth = `${Math.min(rect.width, width)}px`;
  popup.style.maxHeight = `${height}px`;
  popup.style.overflowY = 'auto';
  const below = Math.max(0, top + height - rect.bottom - gap);
  const above = Math.max(0, rect.top - gap - top);
  const upwards = popup.offsetHeight > below && above > below;
  popup.style.maxHeight = `${Math.min(height, upwards ? above : below)}px`;
  popup.style.left = `${Math.max(left, Math.min(rect.left, left + width - popup.offsetWidth))}px`;
  popup.style.top = `${Math.max(top, Math.min(upwards ? rect.top - gap - popup.offsetHeight : rect.bottom + gap, top + height - popup.offsetHeight))}px`;
}
