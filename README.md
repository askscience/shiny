# Shiny — AI Sphere

A Rust-based AI sphere backend with a plugin system. The core binary is the orb: a conversational agent driven by Ollama, with voice (Vosk STT + Supertonic TTS), web search, and an artifact dock. Everything domain-specific (trips, GPS, diaries, navigation) ships as **plugins** — installable via `.zip` or `.tar.gz` upload through the admin API.

See [`PLUGINS.md`](./PLUGINS.md) for the full plugin authoring guide.

## Features

- **Trip Management** — Create, track, start, and end trips
- **GPS Tracking** — Real-time position logging via GPSD daemon with mock fallback
- **OpenStreetMap Integration** — Geocoding, reverse geocoding, routing, and POI search
- **AI-Powered Diary** — Auto-generates Markdown travel diaries using Ollama (gemma4:31b-cloud)
- **Web Search** — DuckDuckGo search with optional AI summarization
- **AI Chat** — Conversational agent aware of your travel history and diary entries
- **Silent Diary Cron** — Daily auto-generation at a configurable time
- **Bearer Token Auth** — Simple but effective authentication
- **Web Navigator UI** — Map-first driving interface with voice AI companion

## Quick Start

### Prerequisites

- Rust 1.75+ (edition 2021)
- SQLite (bundled automatically)
- Python 3.9+ with `pip install 'supertonic[serve]'` (for TTS)
- [Ollama](https://ollama.com) with `gemma4:31b-cloud` (optional — AI features degrade gracefully)
- [GPSD](https://gpsd.io) running on `localhost:2947` (optional — falls back to mock data)
- ~500MB disk for Supertonic ONNX + ~50MB per Vosk language

### Setup

```bash
git clone <repo>
cd shiny

# Configure environment
cp .env .env.local
# Edit .env.local to match your setup

# Terminal 1: Supertonic TTS sidecar
./voice/start_supertonic.sh

# Terminal 2: API + Web UI
RUST_LOG=info cargo run
```

The server starts on `http://0.0.0.0:8080` by default. Open it in a browser (HTTPS required for mic on non-localhost).

Or set `AUTO_START_SUPERTONIC=true` in `.env` to spawn the TTS sidecar automatically.

## Web Navigator UI

The built-in web interface (`web/`) provides a **map-first driving navigator**:

- **OpenStreetMap** follows your GPS with heading rotation
- **Central sphere** — short tap for one AI voice turn, long press for continuous conversation
- **Vosk STT** — offline speech recognition in the browser (models auto-download by language)
- **Supertonic 3 TTS** — on-device text-to-speech via local sidecar ([Hugging Face model](https://huggingface.co/Supertone/supertonic-3))
- **AI artifacts** — slide-up cards for monuments, POIs, routes, and travel plans

### Voice interaction

| Gesture | Action |
|---|---|
| Short tap sphere | Listen once → AI reply spoken aloud |
| Long press sphere | Continuous conversation until tap again |
| ⚙ Settings | Change language (auto-downloads Vosk model) |

Language defaults to your browser/system locale on first launch.

### New API endpoints

| Method | Path | Description |
|---|---|---|
| POST | `/api/agent` | AI agent with tool execution + artifacts |
| POST | `/api/tts` | Supertonic TTS proxy (returns WAV) |
| GET | `/api/voice/status` | Vosk + Supertonic readiness |
| POST | `/api/voice/download` | Download Vosk model for language |
| GET | `/api/voice/models/vosk/{lang}.tar.gz` | Serve Vosk model to browser |
| GET | `/api/voice/languages` | Supported languages |
| GET | `/api/trips/active` | Current active trip |

### Configuration (.env)

| Variable | Default | Description |
|---|---|---|
| `SERVER_HOST` | `0.0.0.0` | Bind address |
| `SERVER_PORT` | `8080` | HTTP port |
| `DATABASE_URL` | `sqlite:data/traveler.db` | SQLite database path |
| `OLLAMA_URL` | `http://127.0.0.1:11434` | Ollama API base URL |
| `OLLAMA_MODEL` | `gemma4:31b-cloud` | Ollama model name |
| `GPSD_HOST` | `127.0.0.1` | GPSD daemon host |
| `GPSD_PORT` | `2947` | GPSD daemon port |
| `DIARY_AUTO_GENERATE` | `true` | Enable daily diary cron |
| `DIARY_GENERATE_TIME` | `21:00` | Diary generation time (HH:MM) |
| `LOG_LEVEL` | `info` | Logging verbosity |
| `SUPERTONIC_URL` | `http://127.0.0.1:7788` | Supertonic TTS sidecar URL |
| `SUPERTONIC_VOICE` | `M1` | Default TTS voice preset |
| `VOSK_MODELS_DIR` | `data/vosk-models` | Downloaded Vosk model storage |
| `AUTO_START_SUPERTONIC` | `false` | Spawn Supertonic sidecar on startup |
| `WEB_DIR` | `web` | Static web UI directory |

## Architecture

```
┌─────────────┐     ┌──────────────┐     ┌─────────────────┐
│  Client App  │────▶│  Axum REST   │────▶│    SQLite DB     │
│  (curl, etc) │◀────│  Server      │◀────│  (sqlx)          │
└─────────────┘     └──────┬───────┘     └─────────────────┘
                           │
              ┌────────────┼────────────┬──────────────┐
              ▼            ▼            ▼              ▼
        ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐
        │  GPSD    │ │  OSM     │ │  Ollama  │ │  Duck    │
        │  daemon  │ │Nominatim │ │  gemma4  │ │  DuckGo  │
        │ :2947    │ │ + OSRM   │ │ :11434   │ │  API     │
        └──────────┘ └──────────┘ └──────────┘ └──────────┘
                           │
                    ┌──────┴──────┐
                    │  diaries/    │
                    │  *.md files  │
                    └─────────────┘
```

## Project Layout

```
src/
├── main.rs              # Server bootstrap, GPSD init, diary cron
├── config.rs            # Configuration from environment
├── errors.rs            # Unified error types & JSON responses
├── auth/mod.rs          # Bearer token auth middleware
├── db/mod.rs            # SQLite pool & migrations
├── models/
│   ├── traveler.rs      # Traveler, RegisterRequest, LoginRequest, AuthResponse
│   ├── trip.rs          # Trip, TripStats, CreateTripRequest
│   ├── location.rs      # Location point, LocationSubmit
│   └── diary.rs         # DiaryEntry, ChatRequest
├── api/
│   ├── mod.rs           # Router assembly (22 endpoints)
│   ├── auth.rs          # POST /auth/register, /auth/login
│   ├── travelers.rs     # GET/PUT /travelers/me
│   ├── trips.rs         # CRUD + start/end/stats + map endpoints
│   ├── locations.rs     # POST/GET locations + trip route
│   ├── diary.rs         # List, get-by-date, search, generate
│   ├── chat.rs          # POST chat with diary context
│   └── search.rs        # POST web search
└── services/
    ├── ollama.rs        # HTTP client to Ollama LLM
    ├── web_search.rs    # DuckDuckGo Lite API client
    ├── gpsd.rs          # GPSD daemon client + mock fallback
    ├── osm.rs           # Nominatim, OSRM, Overpass API client
    └── diary_gen.rs     # Auto-generate diary entries via Ollama
```

## Tech Stack

| Component | Crate |
|---|---|
| HTTP Framework | [axum 0.7](https://crates.io/crates/axum) |
| Database | [sqlx 0.8](https://crates.io/crates/sqlx) + SQLite |
| HTTP Client | [reqwest](https://crates.io/crates/reqwest) |
| Async Runtime | [tokio](https://crates.io/crates/tokio) |
| Serialization | [serde](https://crates.io/crates/serde) + [serde_json](https://crates.io/crates/serde_json) |
| Date/Time | [chrono](https://crates.io/crates/chrono) |
| Auth | [sha2](https://crates.io/crates/sha2) + [uuid](https://crates.io/crates/uuid) |
| GPSD Protocol | Direct TCP (no external lib dependency) |
| CORS | [tower-http](https://crates.io/crates/tower-http) |

## API Overview

| Method | Path | Auth | Description |
|---|---|---|---|
| POST | `/api/auth/register` | No | Register new traveler |
| POST | `/api/auth/login` | No | Login, get Bearer token |
| GET | `/api/travelers/me` | Yes | Get profile |
| PUT | `/api/travelers/me` | Yes | Update profile |
| POST | `/api/trips` | Yes | Create trip |
| GET | `/api/trips` | Yes | List trips |
| GET | `/api/trips/{id}` | Yes | Get trip |
| PUT | `/api/trips/{id}` | Yes | Update trip |
| POST | `/api/trips/{id}/start` | Yes | Start trip |
| POST | `/api/trips/{id}/end` | Yes | End trip + generate diary |
| GET | `/api/trips/{id}/stats` | Yes | Trip statistics |
| POST | `/api/locations` | Yes | Submit GPS point |
| GET | `/api/locations` | Yes | Query locations |
| GET | `/api/trips/{id}/route` | Yes | Trip route |
| GET | `/api/map/search` | Yes | Geocode address |
| GET | `/api/map/reverse` | Yes | Reverse geocode |
| GET | `/api/map/route` | Yes | Get route |
| GET | `/api/map/poi` | Yes | Nearby POIs |
| GET | `/api/diary` | Yes | List diary entries |
| GET | `/api/diary/{date}` | Yes | Get diary entry |
| GET | `/api/diary/search` | Yes | Search diary |
| POST | `/api/diary/generate` | Yes | Force diary generation |
| POST | `/api/chat` | Yes | Chat with AI |
| GET | `/api/chat/history` | Yes | Chat history |
| POST | `/api/search` | Yes | Web search |

## Full API Reference

See [API_DOCS.md](./API_DOCS.md) for complete endpoint documentation, request/response examples, and error codes.

## Diary Format

Auto-generated diary entries follow a Markdown list format:

```markdown
# 2026-05-25 — Paris, France

- **Louvre Museum** (48.8606, 2.3376): Visited at 10:30. Crowded but incredible.
- **Café de Flore** (48.8539, 2.3327): Lunch at 13:00. Excellent croissants.
- **Eiffel Tower** (48.8584, 2.2945): Arrived 15:45. Walked 2.3 km from café.

*Total distance: 8.7 km. Weather: Sunny, 22°C.*
```

## Graceful Degradation

| Service | If Unavailable |
|---|---|
| Ollama | Chat/diary/search return errors; app runs normally |
| Supertonic | TTS fails; STT still works |
| GPSD | Falls back to mock GPS data (Paris, random drift) |
| Nominatim/OSRM | Map endpoints return errors |
| DuckDuckGo | Search returns empty results |

## Development

```bash
# Run with verbose logging
RUST_LOG=debug cargo run

# Build release
cargo build --release

# Check compilation without running
cargo check
```
