# Mail — window surface

[`web/plugin.js`](../web/plugin.js), class prefix `mail-*`.

```js
export default {
  name: 'mail',
  mount: mountMailTile,
  unmount: unmountMailTile,
  getElement: getMailTileElement,
  wireEvents: wireMailEvents,
  contextMenu: mailContextMenu,
};
```

## Layout

The window wears the editorial skin shared by Dusk and Ink (after the
`web/theme-lab/v4/` Mail mockup): hairlines instead of panels, small mono
capitals for labels and metadata, grey pill buttons with a trailing arrow.

- **Chrome** (`data-window-bar`, merged with the window controls by
  `ensureWindowChrome` in `web/js/tiles.js`): account menu, the current folder
  name (`.mail-chrome-title`), mono status labels, compose / sync / account
  actions.
- **Folder rail**: mono `MAILBOX` eyebrow, account name and address, folders as
  plain rows (glyph, name, unread count in mono accent; the active folder is a
  grey pill), and a Mail accounts pill.
- **Message list** (`.mail-scroll`, one scroller): a large folder title
  (`.mail-hero`, display font 34px) with a mono count (`2 NEW · 14 MESSAGES`),
  the search field, then 58px hairline rows — unread dot (accent), sender,
  subject + preview on one ellipsised line, mono time. When the list is
  narrower than 520px (reader open, small window) each row folds onto two
  lines (container query `mail-list`).
- **Reader**: takes a column only while a message is selected
  (`.mail-shell.has-reader`); subject head, Reply / Forward / Delete pills and a
  close button, mono From / To / Date keys, HTML or plain body, attachment pills.
  Its actions row is built once per shell and is the reader's pane bar
  (`data-pane-bar`), so it outlives the message re-renders.
- **Footer**: mono folder + message count, cache note and connection status.
- **Action bar** (`actionBar`, the window's last child): Compose, shown on a
  phone only.

### Narrow windows and phones

The list and the reader are a `paneStack` over `.mail-shell` (the rail is a
non-pane sibling). In a window up to 700px wide, or on a phone, the shell is a
column (`.mail-shell.is-stacked`): the folder rail is a strip across the top,
the list is the floor and picking a message slides the reader in over it with
the shared Back at the start of its actions row (its own Close hides there);
Back, Escape, Android's Back and the edge swipe close the message. Every
button in the window, the composer and the account manager has an icon, so a
phone shows icons alone.

### Large-title handover

`syncScroll()` writes `--sc` (0…1 = `scrollTop / 56` of `.mail-scroll`) on the
window element. CSS uses it to fade the large title out
(`opacity: calc(1 - var(--sc))`), the bar's folder name in (`opacity: var(--sc)`),
and the merged header's hairline and tint in — the header is targeted as
`.mail-tile > .tile-header--merged`, so no hook in `tiles.js` is needed. With no
list (onboarding) `--sc` stays 1, i.e. an ordinary header.

### Motion

Fresh rows (a folder loaded, a search answered) cascade in through the shared
`listReveal(rows)` from `web/ui/index.js`, which plays the user's motion language;
re-renders for selection or read state do not replay it.

Onboarding (while no account is verified), the account manager and the composer
reuse the same grammar. Styles live in core's `web/css/tiles.css` (the Mail
section, keyed by the `mail-*` prefix, the stacked layout included); the
remaining narrow-window rules live in `web/ui/responsive.css`; strings in
`web/i18n/{en,it}.json`.

## Notifications

`mount()` polls the account(s) and raises a **"New mail"** notification when the
unread count grows; `unmount()` stops the poll. Use the core `notify()` helper —
see [notifications](../../../PLUGINS.md).

## Links in messages

An HTML message renders in a sandboxed `srcdoc` frame that cannot reach the app,
so in the kiosk its links are carried out by a click bridge: `mailFrameDocument()`
strips the message's own scripts, inline handlers and unsafe hrefs (so the frame
still runs nothing but the bridge), adds a script that reports clicks with
`postMessage`, and the window routes the URL through core's `openUrl()`
(`web/js/linkRouter.js`) — the page opens in the browser plugin, not in the
reader pane. Outside the kiosk the raw message renders exactly as before. See
[opening links](../../../PLUGINS.md) for the link-handler contract.

## Integration

- `wireMailEvents` reacts to agent actions (e.g. an AI-sent message, a sync).
- `mailContextMenu()` contributes compose/reply/sync/delete entries.

## Development

The app serves the installed copy — `data/plugins/mail/web/`, or
`PLUGINS_DIR/mail/web/` when a same-named copy exists there (it overrides the
baseline) — never `plugins/mail/web/`; copy or reinstall after editing.
