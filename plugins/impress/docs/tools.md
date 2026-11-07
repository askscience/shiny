# Impress — agent tools

| Tool | Params | Returns / behaviour |
|---|---|---|
| `slide_create` | `{ title?, theme?, slides?: [slide] }` | Creates **and fills** a deck. Returns `{ deck_id, title, theme, count }`. |
| `slide_read` | `{ deck_id? }` | `{ deck_id, title, theme, slides, count, hint }` — `slides` is the array you write back. |
| `slide_write` | `{ deck_id?, slides, title?, theme? }` | Replaces the **entire** slide list. Returns `{ deck_id, title, count }`. |
| `slide_edit` | `{ deck_id?, index?, slide }` | Changes one slide (0-based `index`; omitted or out of range appends). Returns `{ deck_id, title, count }`. |
| `slide_list` | `{}` | `{ presentations: [{ deck_id, title, slide_count, updated_at }], count }`. |
| `slide_delete` | `{ deck_id, confirm: true }` | Deletes a deck. |

Without `deck_id`, every tool but `slide_delete` targets the most recently
updated deck. A **slide** object: `{ layout, title, subtitle, bullets, columns, body,
attribution, notes, transition, reveal }`.

## Rules (from [`skills/impress.md`](../skills/impress.md))

- When asked for a presentation, **put real slides in it before replying**;
  never leave an empty deck.
- **Always animate the deck**: set `"transition":"fade"` (or `slide`/`push`/
  `zoom`) on every slide and `"reveal":"bullets"` on `content`/`two-column`
  slides, in the same call that writes the text. A deck with no motion is
  unfinished.
- Always pass the `deck_id`; prefer `slide_edit` for one slide over rewriting
  the whole deck.
- Keep slides scannable (short titles, 3–6 bullets, ~6–10 words each) and vary
  layouts (`section`, `quote`, `two-column`).
- Never `slide_delete` unless explicitly asked (and `confirm: true`).

## Registration

Wrapped with `bridged(..)` in [`src/plugin.rs`](../src/plugin.rs).
