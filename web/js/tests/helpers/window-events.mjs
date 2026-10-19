// Adds the `window` event methods the UI library registers at import time to
// the i18n harness's stub. Import after `i18n.mjs` and before modules that pull
// in `web/ui/`.
globalThis.window.addEventListener = () => {};
globalThis.window.removeEventListener = () => {};
