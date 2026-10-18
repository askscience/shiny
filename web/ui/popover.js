const placements = new WeakMap();

/**
 * Park a menu that hangs off a HUD chip: under it, or ABOVE it when the menu
 * does not fit below and there is more room over it. A top bar docked at the
 * bottom edge needs that flip — otherwise every menu opens off-screen.
 *
 * `align` is the anchor's edge the menu lines up with ('left' or 'right'), and
 * the menu keeps its own width; only its height is capped to the room it is
 * given, so a long list scrolls instead of spilling out of the viewport.
 */
export function placeMenu(popup, anchor, { gap = 8, align = 'left', minHeight = 220 } = {}) {
  const rect = anchor.getBoundingClientRect();
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const margin = 12;
  const bottom = rect.bottom + gap;
  const width = popup.offsetWidth;
  const left = align === 'right'
    ? Math.max(margin, Math.min(rect.right - width, vw - width - margin))
    : Math.max(margin, Math.min(rect.left, vw - width - margin));
  // Room inside the viewport under the anchor, and over it.
  const below = vh - bottom - margin;
  const above = rect.top - gap - margin;
  const upwards = popup.offsetHeight > below && above > below;
  popup.style.left = `${left}px`;
  popup.style.top = `${upwards
    ? Math.max(margin, rect.top - gap - Math.min(popup.offsetHeight, above))
    : bottom}px`;
  popup.style.maxHeight = `${Math.max(minHeight, upwards ? above : below)}px`;
}

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
