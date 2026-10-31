# Impress — window surface

[`web/plugin.js`](../web/plugin.js), class prefix `impress-*`.

```js
export default {
  name: 'impress',
  mount: mountImpressTile,
  unmount: unmountImpressTile,
  getElement: getImpressTileElement,
  wireEvents: wireImpressEvents,
  contextMenu: impressContextMenu,
};
```

## Layout

- **Window bar** (merged with the window controls, no glass plate at rest):
  deck menu, the title, a mono meta (`5 SLIDES · AURORA · SAVED 14:02`, or
  `UNSAVED` / `SAVING`), the theme select (hairline-underlined, translated
  names), a grey **New** pill and the **Present →** pill. The file actions
  (new, import `ui/folder-open`, save now `ui/save`, Save to Documents
  `ui/upload`, delete) live in the deck menu's footer
  (`appendMenuFooter` / `showDocumentMenu` from `web/js/officeMenu.js`; rows
  arrive with `listReveal`).
- **Strip**: thumbnails with a mono number beside each 16:9 frame; the current
  slide is an ink hairline (no accent glow). The thumbnails cascade in with
  `listReveal` when a deck opens — the 300 ms refresh while typing redraws
  without motion.
- **Stage**: the current slide in a hairline frame (no float shadow), edited in
  place; it enters with `cardEnter` when the selected slide changes or a deck
  opens.
- **Inspector**: mono uppercase labels over hairline-underlined selects
  (layout / transition / bullets, with translated option names) and speaker
  notes; slide operations are small grey pills (Add slide, move, delete).
- **Present mode** advances slides, honouring `transition` (fade/slide/push/
  zoom) and `reveal: "bullets"` builds. The overlay is always dark: it derives
  its colours from `--present-bg` (`var(--black)`) and `--present-ink`; the
  counter is mono (`01 / 05`).

Styles: `web/css/tiles.css`, "Impress plugin window".

## Integration

- `wireImpressEvents` reacts to agent actions so AI edits appear live.
- `impressContextMenu(ctx)` contributes deck entries.
- Import/export uses the `.odp` codec.

## Development

The app serves the installed copy at `data/plugins/impress/web/`; copy or
reinstall after editing.
