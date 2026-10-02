# YouTube — REST routes

All routes are `auth` and registered in [`src/plugin.rs`](../src/plugin.rs).

| Method | Path | Query | Purpose |
|---|---|---|---|
| `GET` | `/api/youtube/search` | `?q=&limit=` | Search results. |
| `GET` | `/api/youtube/suggest` | `?video_id=&title=&channel=&query=&limit=` | Recommendations. |
| `GET` | `/api/youtube/categories` | — | The user's topic chips. |
| `GET` | `/api/youtube/home` | — | Homepage shelves ("For you" per category). |

Responses use `{ "success": true, "data": … }`. There is no database; the home
profile is derived from searches/watches.
