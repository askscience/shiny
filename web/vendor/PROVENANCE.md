# Vendored third-party assets

These files are committed so the UI depends only on itself: a CDN compromise
(or an origin-level network attacker on a plain-HTTP deployment) can no longer
inject JavaScript into the privileged desktop origin.

| Path | Version | Source | SHA-256 |
|---|---|---|---|
| `leaflet/leaflet.js` | 1.9.4 | `https://unpkg.com/leaflet@1.9.4/dist/leaflet.js` | `db49d009c841f5ca34a888c96511ae936fd9f5533e90d8b2c4d57596f4e5641a` |
| `leaflet/leaflet.css` | 1.9.4 | `https://unpkg.com/leaflet@1.9.4/dist/leaflet.css` | `a7837102824184820dfa198d1ebcd109ff6d0ff9a2672a074b9a1b4d147d04c6` |
| `leaflet/images/*` | 1.9.4 | `https://unpkg.com/leaflet@1.9.4/dist/images/` | — |
| `marked/marked.min.js` | 12.0.2 | `https://cdn.jsdelivr.net/npm/marked@12.0.2/marked.min.js` | `15fabce5b65898b32b03f5ed25e9f891a729ad4c0d6d877110a7744aa847a894` |

To update: download the new version, update this table, and re-run the UI test
suite. Do not load these from a CDN again.
