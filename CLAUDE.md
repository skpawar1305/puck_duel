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

There is no JavaScript/Svelte test suite. Rust unit tests (physics constants, config sanity checks) live in `src-tauri/src/config.rs`:

```bash
cd src-tauri && cargo test
```

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

- `game.rs` — core game state machine; runs a tokio 60 Hz loop, emits `RenderState` to `Game.svelte` via Tauri Channel; exposes `start_game`, `stop_game`, `pause_game`, `resume_game`, `set_pointer`
- `physics.rs` — collision detection for paddles, walls, corners, and goal posts; AI opponent logic
- `config.rs` — all physics/gameplay constants (`TABLE_WIDTH`, `MAX_SPEED`, `FRICTION`, etc.) grouped into sub-modules `ai`, `network`, `interpolation`, `audio`; contains Rust unit tests; reads `MATCHBOX_SIGNALING_URL` from `src-tauri/.env` at compile time via `build.rs`
- `transport.rs` — WebRTC P2P via `matchbox_socket`; exposes `host_online`, `join_online`, `reset_transport`, `get_room_id`, `cancel_online`
- `udp_transport.rs` — LAN UDP transport + mDNS auto-discovery; exposes `start_udp_host`, `connect_udp_client`, `host_send_msg`, `client_send_msg`, `get_local_ips`, `start_discovery`, `stop_discovery`
- `lib.rs` — Tauri builder, registers all commands and managed state (`WebRtcTransportState`, `GameEngine`, `UdpState`)

### Networking

Two multiplayer modes:
1. **LAN**: mDNS auto-discovery on same Wi-Fi, zero-config
2. **Online**: WebRTC P2P over internet via matchbox signaling server (`MATCHBOX_SIGNALING_URL`); uses 4-digit room codes

**Split authority model**: the side that "owns" the puck runs physics authoritatively; the other dead-reckons.
- Host owns puck when `puck.y >= TABLE_HEIGHT/2`; client owns it on their half
- Authority switches at the midline with an `AUTH_HYSTERESIS` band to prevent rapid flipping
- On authority gain, the new owner blends in the last-received velocity to preserve momentum

### Tauri IPC

- Frontend → Rust: `invoke(...)` from `@tauri-apps/api/core` (pointer input is fire-and-forget)
- Rust → Frontend (render loop): Tauri `Channel` streams `RenderState` at 60 Hz — **not** Tauri events
- Rust → Frontend (transport events): Tauri events via `listen(...)` from `@tauri-apps/api/event` (e.g. `peer-connected`, `udp-msg-received`)

## Key Conventions

- **Svelte 5 runes only**: use `$state`, `$derived`, `$derived.by`, `$props`, `$effect` — not legacy `$:` reactive declarations or stores
- **2D canvas rendering**: `Game.svelte` draws every frame using `CanvasRenderingContext2D` — there is no Three.js/Threlte/WebGL
- **Physics constants live in `config.rs`**: tune gameplay by editing constants there, not inline in `game.rs` or `physics.rs`
- **`src-tauri/.env`** must exist before building (copy from `.env.example`); sets `MATCHBOX_SIGNALING_URL` at compile time — runtime env vars are ignored
- Positions in network messages send only `[x, z]` (y is fixed)
- `release.keystore` is used for Android signing
- `patch_scene.js` and `patch_scene_2.js` are one-off migration scripts — not part of the build
- Android `minSdkVersion` is 28; app ID is `com.dano.puckduel`
