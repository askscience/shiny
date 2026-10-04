// Boot-splash failsafe.
//
// js/bootScreen.js hides the splash as soon as the app or the sign-in screen is
// ready. If the module graph never loads, this still clears it rather than
// leaving a stuck full-screen overlay. A separate file so a strict
// Content-Security-Policy can forbid inline scripts.
setTimeout(function () {
  var el = document.getElementById('boot-screen');
  if (el) el.classList.add('is-hidden');
}, 15000);
