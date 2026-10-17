// Tag the document with the stored interface language before first paint.
//
// Only the *language code* is applied here — the catalogs are fetched later by
// js/i18n.js while the boot splash still covers the app, so nothing needs to
// be translated this early. Kept as a plain (blocking) script and a separate
// file so the CSP can forbid inline scripts, like early-theme.js.
(function () {
  // Keep in step with RTL_LANGUAGES in i18n.js.
  var RTL = { ar: 1, he: 1, fa: 1, ur: 1, yi: 1, dv: 1, ps: 1, sd: 1, ug: 1 };
  var code = '';
  try {
    // The per-traveller value first (the key is written by preferences.js /
    // i18n.js); the unscoped mirror covers the fresh sign-in screen.
    var traveler = JSON.parse(localStorage.getItem('traveler') || 'null');
    if (traveler && traveler.id) code = localStorage.getItem('ui.locale.' + traveler.id) || '';
    code = code || localStorage.getItem('ui.locale') || '';
  } catch (e) {
    /* fall through to the browser language */
  }
  if (!code) code = navigator.language || 'en';
  code = String(code).toLowerCase();
  var root = document.documentElement;
  root.lang = code;
  root.setAttribute('data-locale', code);
  root.dir = RTL[code.split('-')[0]] ? 'rtl' : 'ltr';
})();
