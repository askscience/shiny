## Browser

Open and read web pages inside the **Peak'd Browser** window. Pages are fetched
through PEAK'D!'s own ad-blocking engine, so ads, trackers and known annoyances
are removed before the page reaches the window.

### Tools

- `browser_open` — Open a page in the Browser window. params: `{ url: string }`.
  Accepts a full URL (`https://example.com`) or a search phrase
  (`"best espresso machine"`), which searches instead.
- `browser_search` — Search the web in the Browser window. params:
  `{ query: string }`. Uses the configured SearXNG instance when the server has
  `SEARXNG_URL` set, otherwise Brave Search.
- `browser_read` — Fetch a page through the ad-blocking engine and return its
  readable text **without** opening the window. params:
  `{ url: string, max_chars?: number }`.

### When to use which

- The user asks to **see** something, or to open/search/watch something →
  `browser_open` or `browser_search` (it also focuses the window).
- You need a page's **contents to answer a question** → `browser_read`.
- For a plain fact you can already answer, do not call a tool at all.

### Notes

- `browser_read` returns text only: images, layout and interactive elements are
  lost. If the user needs those, open the window instead.
- Pages that render entirely in JavaScript may return little text. Say so
  rather than inventing the content.
- If a page looks wrong, the user can open the window and use **Direct mode**
  to load it without filtering.
