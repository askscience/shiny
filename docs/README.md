# Shiny documentation

This directory is the source of truth for how Shiny works. It is split into a
small number of areas; each area has its own index and cross-links.

> New here? Start with **[Getting started](getting-started.md)** and
> **[Architecture](architecture.md)**. Writing a plugin? Go straight to
> **[Plugins](plugins/README.md)**.

## Contents

### Orientation

| Document | What it covers |
|---|---|
| [Getting started](getting-started.md) | Prerequisites, build, first run, first use. |
| [Architecture](architecture.md) | Processes, crates, data flow, request/agent/plugin lifecycles. |
| [Configuration](configuration.md) | `.env`, environment variables, data directories, defaults. |
| [Crate map](reference/crate-map.md) | Every workspace crate and its role. |
| [Glossary](reference/glossary.md) | Terms used across the docs. |
| [Environment variables](reference/env-vars.md) | Exhaustive variable reference. |

### Core

| Document | What it covers |
|---|---|
| [Agent](core/agent.md) | The Ollama agent loop, tool dispatch, system prompt, stops. |
| [Chat](core/chat.md) | Conversations, message history, memory. |
| [Voice](core/voice.md) | faster-whisper / Vosk STT, Supertonic / Qwen TTS, the voice bar. |
| [Desktop](core/desktop.md) | Window manager, workspaces, layouts, HUD, launcher, gestures. |
| [Web UI](core/web-ui.md) | The frontend module graph and UI component library. |
| [Themes & icons](core/themes-icons.md) | Tokens, accent/gradient, selectable icon sets and KDE curation. |
| [Data & auth](core/data-and-auth.md) | SQLite schema, migrations, users, tokens, Linux-user binding. |
| [Travel](core/travel.md) | Trips, GPS, maps, navigation and diaries in the core. |

### Host integration

| Document | What it covers |
|---|---|
| [Overview](host/README.md) | The common panel contract and graceful degradation. |
| [Audio](host/audio.md) | PipeWire volume/mute/default-device panel. |
| [Bluetooth](host/bluetooth.md) | BlueZ adapter/device panel. |
| [Battery & power](host/battery-power.md) | sysfs battery + logind power actions. |
| [Network](host/network.md) | NetworkManager Wi-Fi/Ethernet panel. |
| [Display & input](host/display-input.md) | Scale, brightness, backlight, Touch Bar, trackpad gestures. |

### API reference

| Document | What it covers |
|---|---|
| [API index](api/README.md) | Conventions, auth, response shapes, full endpoint index. |
| [Auth & profile](api/auth.md) | Register/login, profile, preferences, background, fonts. |
| [Chat & agent](api/chat-agent.md) | Chat, agent (JSON + SSE), search, artifacts, insights. |
| [Voice](api/voice.md) | TTS, STT streaming, model downloads, languages. |
| [Host](api/host.md) | Network, audio, battery, Bluetooth, power, display, Touch Bar. |
| [Travel](api/travel.md) | Trips, locations, map, navigation, diary. |

### Deployment

| Document | What it covers |
|---|---|
| [Overview](deployment/README.md) | Deployment modes and installers. |
| [Sidecars](deployment/sidecars.md) | The Python/native speech sidecars and their launchers. |
| [Multi-user Linux](deployment/multi-user-linux.md) | Linux-user mode, per-user servers, session and greeter. |
| [Remote access](deployment/remote-access.md) | Iroh peer-to-peer and Tailscale Funnel. |
| [Kiosk shell](deployment/kiosk-shell.md) | `peakd` / `peakd-mac`, the native shells and browser views. |
| [T2 Mac](deployment/t2-mac.md) | Speaker DSP, audio period fix, Bluetooth fix, Touch Bar. |

### Plugins

| Document | What it covers |
|---|---|
| [Plugin system](plugins/README.md) | Overview and index. |
| [Architecture](plugins/architecture.md) | Lifecycle, discovery, hot router swap, uninstall. |
| [Authoring](plugins/authoring.md) | Build your first plugin, step by step. |
| [API reference](plugins/reference.md) | Traits, types, manifest, routes, outcomes. |
| [Runtime & ABI](plugins/runtime-abi.md) | The dlopen boundary, `bridged()`, SQLite rules. |
| [Migrations](plugins/migrations.md) | Per-plugin schema and the immutability rule. |
| [Security](plugins/security.md) | Trust model, signatures, the install log. |
| [Troubleshooting](plugins/troubleshooting.md) | Symptom → cause → fix. |

Each bundled plugin also has its own documentation folder at
`plugins/<name>/docs/`.

## Conventions

- **File references** are relative to the repository root (`src/...`, `web/...`)
  unless stated otherwise.
- **Endpoint references** use the live route table in
  [`src/api/mod.rs`](../src/api/mod.rs); when in doubt, that file wins.
- **Defaults** quoted here come from [`src/config.rs`](../src/config.rs).
- Anything marked **planned** does not exist yet.
