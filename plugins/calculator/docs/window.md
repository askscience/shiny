# Calculator plugin — window surface (`web/plugin.js`)

The Calculator window, in the editorial language: a display with the typed
expression in the display face, sized with the window (`clamp(18px, 4.6cqi,
32px)`, a step below the result) over a large, tight display-font result
(rolled up with `rollNumber` whenever a result is set). The expression line
and the history show the operators as the keys do (`× ÷ −`, via
`prettyExpr`); what is evaluated and stored keeps `* / -`. Then a keypad of flat grey pills with
`=` as the single ink pill, a collapsible scientific pad, and a collapsible
hairline history. The body is a pane stack (`paneStack`, see
`docs/core/ui-components.md` § Panes): the pad (display, scientific row,
keypad) is the floor and the history is a pane that slides in over it in a
narrow window (≤ 700px) or on a phone, with a shared Back in its head; a wide
window keeps the history inline under the keypad, or beside it as a 280px
column from 760px. On a phone the keys grow with the window
(`minmax(44px, 1fr)` rows) like a phone calculator; a short landscape screen
scrolls the pad instead. Styles: the "Calculator plugin window" section of
`web/css/tiles.css`.
`=` posts to `/api/calculator/eval`, the same evaluator the AI tool uses.

## Exports

| Export | Signature | Purpose |
|---|---|---|
| `CALCULATOR_PLUGIN` | `= 'calculator'` | Plugin id used in `data-plugin`, focus events, filtering. |
| `mountCalculatorTile()` | `() => HTMLElement` | Creates (once) and returns the tile element. |
| `unmountCalculatorTile()` | `() => void` | Removes the tile and clears module refs. |
| `getCalculatorTileElement()` | `() => HTMLElement \| null` | The live tile, or `null`. |
| `wireCalculatorEvents()` | `() => void` | Idempotent: attaches the `agent:actions` listener. |
| `calculatorContextMenu()` | `() => Array<MenuItem>` | Right-click menu entries for core to splice. |
| default export | `{ name, icon, mount, unmount, getElement, wireEvents, contextMenu }` | Host contract; `icon: 'ui/calculator'`. |

## Mount lifecycle

`mountCalculatorTile()`:

1. If `tileEl` already exists, returns it (singleton).
2. Creates `<section class="tile calculator-tile" data-plugin="calculator">`.
3. Builds the **top bar** (`div.calculator-bar`, marked `dataset.windowBar = ''` so core merges it into the window controls; no plate at rest): a spacer, a History toggle (`ui/history`, `.calculator-history-toggle`) and a Scientific toggle (`ui/scientific`). An "on" toggle is a grey pill fill (`--ed-pill`) with `aria-pressed`, not an accent box.
4. Builds the **body** (`div.calculator-body`, the pane stack's root) and the **pad** (`div.calculator-pad`, the floor pane).
5. Builds the **display** (`div.calculator-display` with `.calculator-display-expr` and `.calculator-display-result`) in the pad.
6. Builds the **scientific pad** (`div.calculator-sci hidden`) via `buildSciPad()`. Hidden by default; `toggleSci()` flips `is-active`.
7. Builds the **keypad** (`div.calculator-keys`) via `buildKeypad()` in five `.calculator-key-row`s.
8. Builds the **history** (`div.calculator-history hidden`): a head marked `data-pane-bar` (eyebrow "HISTORY", the count, a spacer and Clear history as `ui/trash`) over a scrolling `.calculator-history-body`.
9. Hands the pad and the history to `paneStack(body, …)` (`nav`), paints the history state, calls `renderDisplay()` and returns the tile.

`unmountCalculatorTile()` destroys the stack, removes `tileEl` and nulls all
cached nodes (`exprEl`, `resultEl`, `keysEl`, `sciEl`, `historyEl`,
`historyBodyEl`, `historyCountEl`, toggle buttons).

There is **no polling** in this window.

## DOM structure

```
section.tile.calculator-tile[data-plugin=calculator]
├── div.calculator-bar[data-window-bar]
│   ├── div.calculator-bar-spacer
│   ├── button.calculator-bar-btn.calculator-history-toggle (History)
│   └── button.calculator-bar-btn (Scientific)
└── div.calculator-body.ui-panes                (paneStack root; .is-stacked while narrow)
    ├── div.calculator-pad.ui-pane.is-floor     [data-pane=pad]
    │   ├── div.calculator-display
    │   │   ├── div.calculator-display-expr
    │   │   └── div.calculator-display-result
    │   ├── div.calculator-sci.hidden
    │   │   └── div.calculator-key-row > button.calculator-key (×20)
    │   └── div.calculator-keys
    │       └── div.calculator-key-row > button.calculator-key (×5)   (×5 rows)
    └── div.calculator-history.ui-pane          [data-pane=history] (.hidden while wide and closed)
        ├── div.calculator-history-head[data-pane-bar]
        │   ├── (button.ui-pane-back — the stack's, while stacked)
        │   ├── eyebrow "HISTORY", eyebrow count
        │   ├── div.calculator-bar-spacer
        │   └── button.calculator-bar-btn.calculator-history-clear (Clear history)
        └── div.calculator-history-body
            └── div.ui-hairline-list.calculator-history-list
                └── button.calculator-history-item
                    ├── span.calculator-history-expr   (display face, 14 px)
                    └── span.calculator-history-res    (display face)
```

### CSS class prefix

All classes are prefixed **`calculator-`**: `calculator-tile`, `calculator-bar`,
`calculator-bar-spacer`, `calculator-bar-btn`, `calculator-history-toggle`,
`calculator-body`, `calculator-pad`, `calculator-display`,
`calculator-display-expr`, `calculator-display-result`, `calculator-sci`,
`calculator-key`, `calculator-key-row`, `calculator-history`,
`calculator-history-head`, `calculator-history-clear`,
`calculator-history-body`, `calculator-history-item`,
`calculator-history-expr`, `calculator-history-res`.
Modifiers: `is-fn`, `is-op`, `is-clear`, `is-equals`, `is-active`, `hidden`;
on the result `is-empty` (muted 0 at rest), `is-preview` (the muted live
answer while typing), `is-long`, `is-error`. The bar's DEG/RAD word is
`calculator-angle`. Plugins
ship no CSS — core styles these class names thematically.

## Behaviour & state

Module state: `expression` (string), `lastResult` (number-as-string or `null`),
`sciMode`, `historyOpen`, `nav` (the `paneStack` handle).

- `input(text)` is every key that types: `continueFromResult` then
  `append`. `append` / `backspace()` / `clearAll()` go through
  `setExpression`, which repaints and schedules the preview. Backspace takes
  a function name with its bracket (`sin(`) and `pi` in one press.
- `continueFromResult(text)`: after `=`, an operator (`+ - * / % ^ !`, and
  `x²`) continues from the answer; anything else (a digit, π, e, `(`, a
  function) starts fresh. Either way the old answer leaves the result line.
- `schedulePreview()`: 90 ms after the last key, `apiEval(…, { preview: true
  })` on the expression with its brackets closed; the answer shows muted
  (`is-preview`) until `=`. An incomplete expression keeps the last preview
  that worked; nothing is written to history.
- `plusMinus()` (±): flips the last answer, or the number or bracket being
  typed (`toggleSign` in `expr.js`: `5` → `-5`, `5+3` → `5+(-3)`, and back).
- DEG / RAD: `angleMode`, kept in `localStorage['calculator.angle']`
  (default DEG), sent with every evaluation; the bar's word toggles it.
- Keyboard: `onKeydown` on `document` while the window is in front
  (`tile--focused`) and focus is not in a text field: digits, `+ - * / ^ % (
  ) ! .` (`,` is `.`, `x` and `:` are `*` and `/`), Enter or `=`, Backspace,
  Delete / Escape to clear (Escape closes the history first).
- `equals()`: closes open brackets (`closeParens`), `apiEval`, clears the expression, sets
  `lastResult` to `result_text`, repaints with `rollNumber`, keeps the
  evaluated expression on the mono line, and refreshes history. On failure it
  shows `Error`, switches the glow to the error token, and `toast`s the
  evaluator's message in the interface language (`errorKind` in `expr.js`
  sorts it into `plugin.calculator.error.*`).
- `updateGlow()` uses `setTileGlow` + `glowGradient` with theme tokens only:
  `--error` on error, the `--accent`/`--accent-2` pair otherwise (a result
  moves the blobs by its magnitude).
- History rows are hairline rows (mono expression, display-face result); they
  play `listReveal` when the panel opens (not on each refresh after `=`).
  Clicking one sets `lastResult` to that result (rolled up) and closes the
  panel.
- `openHistory()` / `closeHistory()` / `toggleHistory()` keep `historyOpen`,
  the toggle's `is-active`/`aria-pressed` and the pane in step: wide, the
  history is shown inline by dropping `hidden`; stacked, `hidden` stays off
  and `nav.show('history')` / `nav.back()` slide the pane. Back, Escape and
  the edge swipe pop the pane through the stack; `onPaneChange` then reads
  `nav.isOpen('history')` back into `historyOpen`.
- `clearHistory()` calls `DELETE /api/calculator/history`, refreshes, toasts.

### Keypad

- `OPERATOR_LABELS` maps display glyphs (`×`, `÷`, `√`, `^`, `%`) to class
  hints. Multi-character labels that are not operators get `is-fn`.
- Base rows cover `C`, Backspace (the `ui/arrow-left` catalog icon, labelled
  `plugin.calculator.key.backspace`), `( ) ÷`, `7 8 9 × %`, `4 5 6 − ^`, `1 2 3 + π`,
  `± 0 . = e`.
- Scientific pad: `sin cos tan √ asin acos atan x² ln log log₂ exp 10ˣ ∛ ! abs
  floor ceil round fact`, each appending the matching `fn(` token (`10ˣ`
  appends `10^`). The angle unit is the bar's DEG/RAD, not a key.
- `expr.js` holds the DOM-free text helpers (`prettyExpr`, `closeParens`,
  `toggleSign`, `negateText`, `errorKind`), tested by `expr.test.mjs`.

## Events

| Event | Direction | Detail / effect |
|---|---|---|
| `agent:actions` | listened (`window.addEventListener`, via `wireCalculatorEvents`) | Filters actions whose `action` matches `/^calculator_/`. Dispatches `plugin:focus` with `{ name: 'calculator' }`; on a successful `calculator_eval` it sets `lastResult` from `data.result_text` and refreshes history when open. |
| `plugin:focus` | dispatched | `CustomEvent('plugin:focus', { detail: { name: CALCULATOR_PLUGIN } })` so core raises the window. |

`wireCalculatorEvents()` guards with a module-level `wired` flag, so repeated
calls are no-ops.

## API calls

All through `apiFetch` from `../../js/api.js` (adds the bearer token; throws on
non-2xx).

| Function | Call | Returns |
|---|---|---|
| `apiEval(expression, { preview })` | `POST /api/calculator/eval` `{ expression, angle, preview }` | `res.data` → `{ expression, result, result_text }`. |
| `apiHistory()` | `GET /api/calculator/history?limit=50` | `res.data` → `{ history }` (defaults to `{ history: [] }`). |
| `apiClearHistory()` | `DELETE /api/calculator/history` | ignores the body. |

## Context menu (`calculatorContextMenu`)

Returns entries core wraps with separators/window management: **Equals**
(disabled when the expression is blank), **Clear all**, a separator,
**Scientific** (checked `sciMode`), **History** (checked `historyOpen`), and
**Clear history** (danger).

## UI library imports

`../../ui/index.js`: `toolbar`, `toolButton`, `controlButton`, `emptyState`,
`toast`, `setTileGlow`, `glowGradient`, `icon`, `eyebrow`, `hairlineList`,
`listReveal`, `rollNumber`, `paneStack`. `../../js/api.js`: `apiFetch`.
