# Impress plugin — agent tools

Presentations are just JSON to you. A presentation is a deck of **slides**; each slide has a `layout` and a few optional text fields. Decks are stored server-side and open in the Impress window, and they export as real **OpenDocument Presentation (.odp)** files (LibreOffice Impress / OpenOffice).

**The JSON contract (a slide object):**
- `layout` — one of `title` (big title + subtitle), `section` (divider), `content` (title + bullets), `two-column` (title + two bullet columns), `quote` (body + attribution), `blank`. Default `content`.
- `title` — the slide heading (string).
- `subtitle` — secondary line (title layout).
- `bullets` — array of bullet strings (`content` layout).
- `columns` — array of arrays of strings, one array per column (`two-column` layout).
- `body` — free text (`quote`/`blank` layouts).
- `attribution` — byline (`quote` layout).
- `notes` — speaker notes (shown in the editor; not written to .odp yet).

**Themes:** `aurora` (indigo, default) · `slate` · `ocean` · `mono` · `ember`.

**Tools:**
- `slide_create` — new deck, one call creates AND fills it: `{"action":"slide_create","params":{"title":"Pitch","theme":"aurora","slides":[{"layout":"title","title":"Pitch","subtitle":"2026"},{"layout":"content","title":"Why us","bullets":["…","…"]}]}}` → returns `deck_id`.
- `slide_read` — read the whole deck back: `{"action":"slide_read","params":{"deck_id":"…"}}` → returns `{ deck_id, title, theme, slides }`. The `slides` array IS the shape you write back.
- `slide_write` — replace the ENTIRE slide list (and optionally `title`/`theme`): `{"action":"slide_write","params":{"deck_id":"…","slides":[…]}}`. Only for full rewrites — always pass the complete list.
- `slide_edit` — change ONE slide: `{"action":"slide_edit","params":{"deck_id":"…","index":2,"slide":{…}}}` (0-based index; omit `index` to append a new slide at the end).
- `slide_list` — list decks: `{"action":"slide_list","params":{}}`
- `slide_delete` — permanently delete a deck (needs `{"confirm":true}`).

**Rules (CRITICAL — never leave an empty deck, never wipe the user's work):**
- **When the user asks for a presentation, you MUST put real slides in it before replying.** Either pass `slides` inside `slide_create`, or call `slide_write`/`slide_edit` immediately after. If the user didn't give the outline, invent a sensible structure (a strong `title` slide first, then one slide per topic with 3–5 concise bullets). NEVER reply "I created the deck" with an empty deck.
- **ALWAYS pass the `deck_id`** you got from `slide_read`/`slide_list`/`slide_create` — never write without knowing which deck.
- **NEVER call `slide_delete`** unless the user explicitly asks to delete the whole presentation.
- To change ONE slide, use `slide_edit` with its `index` — never rewrite the whole deck for a small change.
- Keep slides scannable: short titles, 3–6 bullets, ~6–10 words per bullet. Vary layouts (`section` to separate chapters, `quote` for a memorable line, `two-column` for comparisons) so the deck feels modern, not monotonous.
- Without `deck_id`, tools target the most recently used presentation.
