# Vendored libraries for Capture graphics

Graphics render offline (the renderer and the preview allow no network), so
the libraries a graphic may use ship with the plugin and are inlined into the
page when the graphic asks for them (`<script src="lib/three.js"></script>`,
see `src/editor/render.rs`).

| File | Library | Version | Source | License |
|---|---|---|---|---|
| `three.min.js` | three.js | 0.160.0 (r160, the last classic build: global `THREE`) | `https://registry.npmjs.org/three/-/three-0.160.0.tgz`, `package/build/three.min.js` (tarball sha256 `1ee2f935c4f555814b388e87b5ef78a44856bd2e9d0feb88643a6e193fb42856`) | MIT, `three.LICENSE` |

Local change: the `console.warn` that opens `three.min.js` (saying the
classic build is deprecated) is replaced with `void 0`, so it does not show up in every
render error.
