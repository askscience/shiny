# Calculator plugin — window surface (`web/plugin.js`)

The Calculator window, in the editorial language: a display with a small mono
uppercase expression line over a large, tight display-font result (rolled up
with `rollNumber` whenever a result is set), a keypad of flat grey pills with
`=` as the single ink pill, a collapsible scientific pad, and a collapsible
hairline history. Styles: the "Calculator plugin window" section of
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
3. Builds the **top bar** (`div.calculator-bar`, marked `dataset.windowBar = ''` so core merges it into the window controls; no plate at rest): a spacer, a History toggle (`ui/list`), a Scientific toggle (`ui/scientific`), and Clear history (`ui/trash`). An "on" toggle is a grey pill fill (`--ed-pill`) with `aria-pressed`, not an accent box.
4. Builds the **display** (`div.calculator-display` with `.calculator-display-expr` and `.calculator-display-result`).
5. Builds the **scientific pad** (`div.calculator-sci hidden`) via `buildSciPad()`. Hidden by default; `toggleSci()` flips `is-active`.
6. Builds the **keypad** (`div.calculator-keys`) via `buildKeypad()` in five `.calculator-key-row`s.
7. Builds the **history** (`div.calculator-history hidden`).
8. Calls `renderDisplay()` and returns the tile.

`unmountCalculatorTile()` removes `tileEl` and nulls all cached nodes
(`exprEl`, `resultEl`, `keysEl`, `sciEl`, `historyEl`, toggle buttons).

There is **no polling** in this window.

## DOM structure

```
section.tile.calculator-tile[data-plugin=calculator]
├── div.calculator-bar[data-window-bar]
│   ├── div.calculator-bar-spacer
│   ├── button.calculator-bar-btn (History)
│   ├── button.calculator-bar-btn (Scientific)
│   └── button.calculator-bar-btn (Clear history)
├── div.calculator-display
│   ├── div.calculator-display-expr
│   └── div.calculator-display-result
├── div.calculator-sci.hidden
│   └── div.calculator-key-row > button.calculator-key (×20)
├── div.calculator-keys
│   └── div.calculator-key-row > button.calculator-key (×5)   (×5 rows)
└── div.calculator-history.hidden
    ├── div.calculator-history-head (eyebrow "HISTORY" + count)
    └── div.ui-hairline-list.calculator-history-list
        └── button.calculator-history-item
            ├── span.calculator-history-expr   (mono, uppercase)
            └── span.calculator-history-res    (display face)
```

### CSS class prefix

All classes are prefixed **`calculator-`**: `calculator-tile`, `calculator-bar`,
`calculator-bar-spacer`, `calculator-bar-btn`, `calculator-display`,
`calculator-display-expr`, `calculator-display-result`, `calculator-sci`,
`calculator-key`, `calculator-key-row`, `calculator-history`,
`calculator-history-item`, `calculator-history-expr`, `calculator-history-res`.
Modifiers: `is-fn`, `is-op`, `is-clear`, `is-equals`, `is-active`, `hidden`;
on the result `is-empty` (muted 0 at rest), `is-long`, `is-error`. Plugins
ship no CSS — core styles these class names thematically.

## Behaviour & state

Module state: `expression` (string), `lastResult` (number-as-string or `null`),
`sciMode`, `historyOpen`.

- `append(text)` / `backspace()` / `clearAll()` mutate the expression and
  repaint.
- `continueFromResult(text)`: after `=`, pressing an operator continues from
  the answer (pocket-calculator behaviour); pressing a digit starts fresh.
- `equals()`: trims the expression, `apiEval`, clears the expression, sets
  `lastResult` to `result_text`, repaints with `rollNumber`, keeps the
  evaluated expression on the mono line, and refreshes history. On failure it
  shows `Error`, switches the glow to the error token, and `toast`s the
  message.
- `updateGlow()` uses `setTileGlow` + `glowGradient` with theme tokens only:
  `--error` on error, the `--accent`/`--accent-2` pair otherwise (a result
  moves the blobs by its magnitude).
- History rows are hairline rows (mono expression, display-face result); they
  play `listReveal` when the panel opens (not on each refresh after `=`).
  Clicking one sets `lastResult` to that result (rolled up) and closes the
  panel.
- `clearHistory()` calls `DELETE /api/calculator/history`, refreshes, toasts.

### Keypad

- `OPERATOR_LABELS` maps display glyphs (`×`, `÷`, `√`, `^`, `%`) to class
  hints. Multi-character labels that are not operators get `is-fn`.
- Base rows cover `C`, Backspace (the `ui/arrow-left` catalog icon, labelled
  `plugin.calculator.key.backspace`), `( ) ÷`, `7 8 9 × %`, `4 5 6 − ^`, `1 2 3 + π`,
  `± 0 . = e`.
- Scientific pad: `sin cos tan √ asin acos atan x² ln log log₂ exp deg rad ! abs
  floor ceil round fact`, each appending the matching `fn(` token.

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
| `apiEval(expression)` | `POST /api/calculator/eval` `{ expression }` | `res.data` → `{ expression, result, result_text }`. |
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
`listReveal`, `rollNumber`. `../../js/api.js`: `apiFetch`.
