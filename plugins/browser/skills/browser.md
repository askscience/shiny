## Browser

Open and read web pages inside the **Browser** window. Pages are fetched
through the app's own ad-blocking engine, so ads, trackers and known annoyances
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
- If a page looks wrong because a site needs its trackers, the user can pause ad
  blocking from the window's **shield toggle** on the right of the toolbar (it
  applies to every page until turned back on).
- The window also has a **Downloads** manager (every download from every site,
  with progress and controls; files land in Files → Downloads) and **incognito**
  tabs. You do not need to describe these unless the user asks.
- `browser_read` fetches the page text directly; the ad blocker runs in the
  window for rendered pages, so `browser_read` filtering is best-effort.
- The window's home surface (the Home button, shown when it opens) lists
  **related news** chosen from what the user has recently searched for. Those
  cards are a UI feature, not a tool: do not offer to "open the news shelf" —
  just open or read the pages the user actually asks about.
