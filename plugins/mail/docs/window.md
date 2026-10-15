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

- Account switcher, folder sidebar, message list, and a reader with HTML/plain
  rendering plus a composer.

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

The app serves the installed copy at `data/plugins/mail/web/`; copy or reinstall
after editing.
