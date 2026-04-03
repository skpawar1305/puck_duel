# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Full Tauri dev (frontend + Rust backend together)
npm run tauri dev

# Frontend only (Vite dev server at :1420)
npm run dev

# Type-check Svelte/TypeScript
npm run check
npm run check:watch

# Build frontend only
npm run build

# Bundle desktop app
npm run tauri build

# Build Android AAB (Play Store)
npm run tauri android build

# Build Android APK (sideload)
npm run tauri android build -- --apk
```

There is no test suite.

## Architecture

PuckDuel is a P2P multiplayer air hockey game for Android/desktop built with:
- **Tauri 2** (Rust backend) + **SvelteKit** (Svelte 5, SPA mode) + **TypeScript**
- **Tailwind CSS v4**

SvelteKit uses `adapter-static` in SPA mode (`fallback: "index.html"`) — required for Tauri (no SSR).

### Frontend (`src/`)

- `src/routes/+page.svelte` — single SvelteKit route; manages screens (`menu | host | join | game`) and invokes Tauri commands
- `src/lib/components/Game.svelte` — main game canvas renderer (draws RenderState from Rust at 60 Hz via Tauri Channel)
- `src/lib/components/QRScanner.svelte` — wraps `html5-qrcode` for scanning host IP/room QR codes
- `src/lib/matchbox.ts` — WebRTC/matchbox state management for online multiplayer
- `src/lib/audio.ts` — Web Audio API synth; must call `initAudio()` from a user-gesture handler before playing sounds

### Rust backend (`src-tauri/src/`)

- `lib.rs` — Tauri builder, registers all commands and plugins
- `game.rs` — core game state machine (largest file, ~35KB); runs a 60 Hz tokio loop, streams `RenderState` to frontend via Tauri Channel
- `physics.rs` — collision detection, friction, AI opponent logic
- `transport.rs` — WebRTC P2P via `matchbox_socket` (online multiplayer with 4-digit room codes)
- `udp_transport.rs` — LAN mDNS auto-discovery and UDP transport
- `config.rs` — reads `MATCHBOX_SIGNALING_URL` from compile-time env (set via `src-tauri/.env`)

### Networking

Two multiplayer modes:
1. **LAN**: mDNS auto-discovery on same Wi-Fi, zero-config
2. **Online**: WebRTC P2P over internet via matchbox signaling server (`MATCHBOX_SIGNALING_URL`); uses 4-digit room codes

**Split authority model**: the side that "owns" the puck runs physics authoritatively; the other interpolates.
- Host owns puck when on host's half of table; client owns puck on client's half
- Authority switches at the midline; the new owner adopts last-received velocity to maintain momentum

### Tauri IPC

- Frontend → Rust: `invoke(...)` from `@tauri-apps/api/core`
- Rust → Frontend: Tauri Channel API streams `RenderState` frames at 60 Hz; events use `listen(...)` from `@tauri-apps/api/event`
- Pointer/input events are fire-and-forget from JS to Rust

## Key Conventions

- **Svelte 5 runes only**: use `$state`, `$derived`, `$derived.by`, `$props`, `$effect` — not legacy `$:` reactive declarations or stores
- Positions in network messages send only `[x, z]` (y is fixed)
- `src-tauri/.env` holds `MATCHBOX_SIGNALING_URL`; loaded at compile time via `build.rs` — copy from `src-tauri/.env.example`
- `release.keystore` is used for Android signing
- `patch_scene.js` and `patch_scene_2.js` are one-off migration scripts — not part of the build
- Android `minSdkVersion` is 28; app ID is `com.dano.puckduel`
