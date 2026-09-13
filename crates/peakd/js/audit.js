(function () {
  const out = {};
  const all = [...document.querySelectorAll('*')];
  out.nodes = all.length;

  // Every element that forces a backdrop-blur pass, plus its blur radius.
  const blurs = [];
  for (const el of all) {
    const cs = getComputedStyle(el);
    const bf = cs.backdropFilter || cs.webkitBackdropFilter;
    if (bf && bf !== 'none') {
      const r = el.getBoundingClientRect();
      blurs.push({
        sel: el.id ? '#' + el.id : (el.className && typeof el.className === 'string'
              ? '.' + el.className.trim().split(/\s+/).slice(0, 2).join('.') : el.tagName),
        filter: bf,
        area: Math.round(r.width * r.height),
        visible: r.width > 0 && r.height > 0 && cs.display !== 'none' && cs.visibility !== 'hidden',
      });
    }
  }
  out.backdrop_layers = blurs.length;
  out.backdrop_visible = blurs.filter(b => b.visible).length;
  out.backdrop_total_area = blurs.filter(b => b.visible).reduce((a, b) => a + b.area, 0);
  out.backdrop_top = blurs.filter(b => b.visible).sort((a, b) => b.area - a.area).slice(0, 8);

  // Elements with a non-trivial box-shadow (each needs its own shadow pass).
  let shadows = 0;
  for (const el of all) {
    const s = getComputedStyle(el).boxShadow;
    if (s && s !== 'none') shadows++;
  }
  out.box_shadowed = shadows;

  // Continuously animating elements.
  out.animated = all.filter(el => {
    const cs = getComputedStyle(el);
    return cs.animationName && cs.animationName !== 'none'
      && cs.animationIterationCount === 'infinite';
  }).map(el => (el.id ? '#' + el.id : el.tagName + '.' + String(el.className).split(' ')[0]));

  // Canvas sizes: a canvas backing store larger than its CSS box is wasted work.
  out.canvases = [...document.querySelectorAll('canvas')].map(c => ({
    id: c.id || c.className || 'canvas',
    css: Math.round(c.clientWidth) + 'x' + Math.round(c.clientHeight),
    buffer: c.width + 'x' + c.height,
    ratio: +(c.width / Math.max(1, c.clientWidth)).toFixed(2),
  }));

  out.dpr = window.devicePixelRatio;

  // Optional experiment hook: `window.__auditNoBackdrop = true` before the
  // audit runs strips every backdrop-filter, so the same probe can A/B the
  // cost of the blur passes instead of guessing at it.
  if (window.__auditNoBackdrop) {
    const style = document.createElement('style');
    style.textContent = '*, *::before, *::after { backdrop-filter: none !important; -webkit-backdrop-filter: none !important; }';
    document.head.appendChild(style);
    out.backdrop_disabled = true;
  }

  return JSON.stringify(out);
})()
