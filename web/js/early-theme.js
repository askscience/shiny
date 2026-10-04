// Apply the stored theme synchronously, before first paint.
//
// theme-loader.js swaps the stylesheet links after boot, which would flash a
// different theme first. The installed-theme index is cached by listThemes()
// for the check here. Loaded as a plain (blocking) script from <head>; it is a
// separate file so a strict Content-Security-Policy can forbid inline scripts.
(function () {
  var DEFAULT = 'pitch'; // keep in step with theme-loader.js
  var theme = DEFAULT;
  try {
    var stored = localStorage.getItem('ui.theme.name');
    var list = JSON.parse(localStorage.getItem('ui.theme.list') || 'null');
    var known = Array.isArray(list) && list.length;
    // Trust a stored choice unless the cached index says it is gone.
    if (stored && (!known || list.indexOf(stored) >= 0)) theme = stored;
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
