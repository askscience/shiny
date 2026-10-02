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

## Integration

- `wireMailEvents` reacts to agent actions (e.g. an AI-sent message, a sync).
- `mailContextMenu()` contributes compose/reply/sync/delete entries.

## Development

The app serves the installed copy at `data/plugins/mail/web/`; copy or reinstall
after editing.
