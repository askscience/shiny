# Keyboard — architecture

The virtual keyboard is a **pure surface** plugin: it contributes no agent tools
and no REST routes. Its clickable bar is **chrome-integrated** — the UI lives in
core, not in the plugin folder.

```
plugins/keyboard/
├── plugin.toml         name, api_level, description
├── src/{lib,plugin}.rs  registers nothing (register() is empty)
└── web/icon.svg         identity icon
```

There is no `web/plugin.js` and no `skills/`: the keyboard is not a tile window.
The bar is rendered by core's
[`web/js/keyboard.js`](../../../web/js/keyboard.js) with styles in
`web/css/keyboard.css`, and the HUD toggle is core chrome. The plugin exists so
the keyboard is an installable/activatable unit with an identity icon and a
category.

## Why it is chrome, not a tile

The keyboard bar sits at the **bottom of the screen** and types into whatever
input is focused, across every window. That is chrome behaviour, not a
window/tile, so it does not follow the `mount()/unmount()` window contract.
This is the "chrome surface contract" still on the roadmap in
[PLUGINS.md §20](../../../PLUGINS.md): the goal is for chrome plugins like
`keyboard` (and `traveler`'s map/dock) to ship their surface entirely inside
their own folder.

## Layouts

The keyboard ships 8 language layouts; the user selects one, and taps type into
the focused input.

## Related

[README](README.md) · [window](window.md) ·
[plugin system](../../../docs/plugins/README.md).
