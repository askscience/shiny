# Traveler — window surface

The traveler plugin ships **no `web/plugin.js`**. Its visual surfaces are
**core-hosted**, because the map, dock and HUD predate/extend the plugin and are
shared with the core travel services:

| Surface | Owner | Notes |
|---|---|---|
| Leaflet map window | core | `web/js/map.js`; core supplies a context-menu entry *Center on my position*. |
| Navigator HUD | core | `web/js/navigator.js` + `navigationApi.js`; consumes `NavigationSession` from either side. |
| Saved-places HUD menu | core | Top HUD. |
| Destination insight cards | core | `web/js/insights/*` + `src/services/insights`. |
| **Artifact cards** | plugin | The `show_artifact`/`update_artifact` tools emit cards rendered by the artifact panel/dock. |

The plugin does emit `web/icon.svg` for its identity icon (tray/launcher). The
map window is the "core-hosted window" case described in
[plugin UI conventions](../../../PLUGINS.md) — a plugin can be chrome/window-
integrated rather than owning a tile.

## Artifact cards

A card is an [`Artifact`](../../../crates/shiny-plugin-sdk/src/artifacts.rs)
rendered inside its owning context and saved to the user's artifact store with
the `plugin: "traveler"` tag. Types: `travel_plan`, `site_info`, `poi_list`,
`route_preview`, `monument_info`, `tour_plan`; optional `days[]`, `route`,
`coordinates`, `geometry`, `narrative` and `theme`.

## See also

- [Plugin UI & theming](../../../PLUGINS.md)
- [Travel domain](../../../docs/core/travel.md)
- [Themes & icons](../../../docs/core/themes-icons.md)
