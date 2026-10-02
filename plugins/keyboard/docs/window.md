# keyboard — window / surface

> [Plugin README](./README.md) · [Plugins overview](../../../docs/plugins/README.md)

The keyboard's UI is **core chrome**, implemented in `web/js/keyboard.js` (683
lines) and styled by `web/css/keyboard.css`. It is not a `web/plugin.js` window
surface — there is no tile, no title bar and no context menu. This document is
the surface contract.

## Exports

`web/js/keyboard.js` — a plain ES module (no default export).

| Export | Signature | Purpose |
|---|---|---|
| `KEYBOARD_PLUGIN` | `'keyboard'` | Plugin name constant |
| `initKeyboard()` | `() => void` | One-time init: restores saved layout, wires global events, subscribes to `plugins:changed` |
| `refreshKeyboard()` | `async () => void` | Reads the active set and mounts/unmounts the bar if the plugin's active state changed |
| `toggleKeyboard()` | `() => boolean` | Shows/hides the bar exactly like the HUD toggle; returns `false` when the plugin is not active |

Public consumers: `web/js/touchbar.js` imports `toggleKeyboard` for its `Keys`
button (`case 'keyboard'` at `web/js/touchbar.js:130`), and app startup calls
`initKeyboard()`.

## Mount lifecycle

```
initKeyboard()                         # once, at app start
  ├─ restore layout from localStorage('keyboard.lang.<traveler|default>')
  ├─ wireEvents()
  └─ on 'plugins:changed' → refreshKeyboard()

refreshKeyboard()
  ├─ activeSet = await refreshActivePlugins()
  ├─ nowActive = activeSet.has('keyboard')
  └─ if changed:  mount()  or  unmount()
```

`mount()` (`web/js/keyboard.js:603`) is idempotent (`if (bar) return`):

1. Creates `#keyboard-bar` (`role="group"`, `aria-label="Virtual keyboard"`)
   containing `.keyboard-rows`.
2. Appends it to `document.body`.
3. Calls `createHudButton()`.
4. Marks every existing `input, textarea` with `inputmode="none"`, and starts a
   `MutationObserver` that marks newly added inputs.
5. `render()`s the layout.

`unmount()` (`web/js/keyboard.js:630`) closes the bar, disconnects the observer,
removes `inputmode`/`readonly` from marked inputs, removes `#keyboard-bar` and
the HUD button, and clears state.

## DOM tree

```
body
├─ #keyboard-bar                 (+ .visible when open)
│   └─ .keyboard-rows
│       ├─ .keyboard-row.keyboard-row--desktop   (function row)
│       │   └─ .keyboard-key.keyboard-key--fn.keyboard-key--desktop
│       ├─ .keyboard-row                          (letters, one per layout row)
│       ├─ .keyboard-row                          (bottom row)
│       │   ├─ .keyboard-key.keyboard-key--fn     (lang / sym)
│       │   ├─ .keyboard-key.keyboard-key--space
│       │   └─ .keyboard-key.keyboard-key--accent.keyboard-key--fn  (enter)
└─ #hud-top
    └─ #hud-keyboard-btn.icon-btn                (inserted before #settings-btn)
```

`body` also gains the `.keyboard-open` class while the bar is visible.

### Key classes (prefix `keyboard-`)

| Class | Element |
|---|---|
| `keyboard-row` | One row of keys |
| `keyboard-row--desktop` | The Hyprland-style desktop control row |
| `keyboard-key` | Every key button |
| `keyboard-key--fn` | Function keys (shift, backspace, lang, sym, enter) |
| `keyboard-key--space` | Space bar |
| `keyboard-key--accent` | Enter |
| `keyboard-key--desktop` | Desktop control keys |
| `is-active` | Shift currently engaged |

## Layouts

`LAYOUTS` (`web/js/keyboard.js:27`) declares **8 layouts**:

| Index | `code` | Label | Direction | Notes |
|---|---|---|---|---|
| 0 | `en` | `EN` | ltr | QWERTY |
| 1 | `it` | `IT` | ltr | `è`, `ù` |
| 2 | `es` | `ES` | ltr | `ñ` |
| 3 | `fr` | `FR` | ltr | AZERTY, `è` |
| 4 | `de` | `DE` | ltr | QWERTZ, `ü ö ä` |
| 5 | `ru` | `RU` | ltr | Cyrillic + `shiftRows` |
| 6 | `el` | `ΕΛ` | ltr | Greek + `shiftRows` |
| 7 | `ar` | `ع` | rtl | Arabic, `noShift: true` |

`SYMBOL_ROWS` (`web/js/keyboard.js:105`) is a language-independent `123` page.
Layout cycling is `pressKey('lang')`; the selection is persisted per traveler
under `localStorage['keyboard.lang.<travelerId|default>']`.

The desktop control row exposes fullscreen (`Alt+Enter`), previous/next
workspace (`Alt+,` / `Alt+.`), new/remove workspace (`Alt+N` / `Alt+Shift+N`) and
focus-next-window (`Alt+L`), all routed through `web/js/desktop.js` and
`web/js/fullscreen.js`.

## Input-target binding

An input becomes the keyboard's target when it receives DOM focus and is
editable:

- Match: `HTMLTextAreaElement`, or `HTMLInputElement` whose `type` is one of
  `text, password, email, search, number, tel, url`, or any
  `isContentEditable` element.
- `bindTarget(el)` stores the element and opens the bar.
- Text fields are edited via `selectionStart`/`selectionEnd`; contenteditable is
  edited with `document.execCommand` on desktop and a saved `Range` on touch
  (`insertCharTouch` / `backspaceTouch` / `insertParagraphTouch`), because touch
  contenteditable is blurred to suppress the OS keyboard.
- On touch, every input gets `inputmode="none"` and `readOnly = true` while the
  plugin is active.
- Edits dispatch a bubbling `input` event (`InputEvent` with
  `inputType: 'insertText'`, falling back to a plain `Event`) so app code reacts
  as if the user typed.

## Events handled

All wired once by `wireEvents()` (`web/js/keyboard.js:495`):

| Event | Target | Behaviour |
|---|---|---|
| `focusin` | document | Bind editable targets; close on entering an `IFRAME` unless pinned |
| `focusout` | document | Close when focus leaves the target (skips when `pinned` or focus moves to another editable) |
| `blur` | window | Close when the parent blurs into a same-tab iframe |
| `pointerdown` (capture) | document | Close when tapping outside the bar/HUD unless `pinned` |
| `keydown` | document | `Escape` closes the bar |
| `selectionchange` | document | Keep the saved contenteditable `Range` fresh |
| `plugins:changed` | window | `refreshKeyboard()` |

The HUD button (`#hud-keyboard-btn`) toggles a **pinned** state: `pinned = true;
open()`. While pinned, blur/outside-tap/iframe handlers will not auto-close it.

## Open / close

| Function | Effects |
|---|---|
| `open()` | `visible = true`, add `.visible` + `body.keyboard-open`, `aria-pressed="true"` |
| `close()` | `visible = false`, `pinned = false`, stop backspace repeat, remove classes, `aria-pressed="false"` |

## API calls

None. The keyboard is entirely client-side — it causes no `/api/*` traffic
beyond the core's `GET /api/plugins/active` refresh performed by
`refreshActivePlugins()`.

## State & persistence

| State | Meaning |
|---|---|
| `bar`, `rowsEl` | DOM roots |
| `hudBtn` | Top-bar toggle |
| `active` / `visible` / `pinned` | Plugin active / bar open / manually toggled open |
| `target`, `activeRange` | Focused editable + saved contenteditable caret |
| `layoutIdx`, `shift`, `symbols` | Render state |
| `markedInputs` | Inputs given `inputmode="none"`/`readOnly` |
| `repeatTimer` | Backspace auto-repeat (400 ms delay, 60 ms interval) |

Persisted: the active layout code only (`keyboard.lang.<travelerId|default>` in
`localStorage`). Nothing is sent to the server.
