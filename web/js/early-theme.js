// Apply the stored theme synchronously, before first paint.
//
// theme-loader.js swaps the stylesheet links after boot, which would flash a
// different theme first. The installed-theme index is cached by listThemes()
// for the check here. Loaded as a plain (blocking) script from <head>; it is a
// separate file so a strict Content-Security-Policy can forbid inline scripts.
(function () {
  var DEFAULT = 'dusk'; // keep in step with theme-loader.js
  // Retired themes → their replacement (RETIRED_THEMES in theme-loader.js).
  var RETIRED = { pitch: 'ink', noir: 'ink', neumorphic: 'ink', light: 'dusk', 'neumorphic-light': 'dusk' };
  var theme = DEFAULT;
  try {
    var stored = localStorage.getItem('ui.theme.name');
    var list = JSON.parse(localStorage.getItem('ui.theme.list') || 'null');
    var known = Array.isArray(list) && list.length;
    if (stored && RETIRED[stored]) {
      // The cached index predates the retirement, so it can't vouch for the
      // replacement; the replacement always ships.
      theme = RETIRED[stored];
    } else if (stored && (!known || list.indexOf(stored) >= 0)) {
      // Trust a stored choice unless the cached index says it is gone.
      theme = stored;
    }
  } catch (e) {
    /* keep the default */
  }
  document.documentElement.setAttribute('data-theme', theme);
  var set = function (id, href) {
    var el = document.getElementById(id);
    if (el) el.setAttribute('href', href);
  };
  set('theme-tokens', '/themes/' + theme + '/tokens.css');
  set('theme-components', '/themes/' + theme + '/components.css');
})();
