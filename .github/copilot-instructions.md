# Copilot Instructions

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

# Run Rust unit tests (physics constants, config sanity checks)
cd src-tauri && cargo test
```

There is no JavaScript/Svelte test suite. Rust tests live in `src-tauri/src/config.rs`.

## Architecture

PuckDuel is a P2P multiplayer air hockey game for Android/desktop built with:
- **Tauri 2** (Rust backend) + **SvelteKit** (Svelte 5, SPA mode) + **TypeScript**
- **Tailwind CSS v4**
- No SSR — SvelteKit uses `adapter-static` with `fallback: "index.html"` (required for Tauri)

### Frontend (`src/`)

- `src/routes/+page.svelte` — single SvelteKit route; manages screens (`menu | host | join | game`) and invokes Tauri commands
- `src/lib/components/Game.svelte` — main game canvas renderer; receives `RenderState` from Rust at 60 Hz via a Tauri `Channel` and draws to an HTML `<canvas>` using the 2D Context API
- `src/lib/components/QRScanner.svelte` — wraps `html5-qrcode` for scanning host IP/room QR codes
- `src/lib/matchbox.ts` — WebRTC/matchbox state management for online multiplayer
- `src/lib/audio.ts` — Web Audio API synth; `initAudio()` **must** be called from a user-gesture handler before any sound plays

### Rust backend (`src-tauri/src/`)

- `game.rs` (~830 lines) — core game state machine; runs a tokio 60 Hz loop, emits `RenderState` to `Game.svelte` via Tauri Channel; exposes `start_game`, `stop_game`, `pause_game`, `resume_game`, `set_pointer`
- `physics.rs` — collision detection for paddles, walls, corners, and goal posts; AI opponent logic
- `config.rs` — all physics/gameplay constants (`TABLE_WIDTH`, `MAX_SPEED`, `FRICTION`, etc.); grouped into sub-modules `ai`, `network`, `interpolation`, `audio`; contains Rust unit tests
- `transport.rs` — WebRTC P2P via `matchbox_socket`; exposes `host_online`, `join_online`, `reset_transport`, `get_room_id`, `cancel_online`
- `udp_transport.rs` — LAN UDP transport + mDNS auto-discovery; exposes `start_udp_host`, `connect_udp_client`, `host_send_msg`, `client_send_msg`, `get_local_ips`, `start_discovery`, `stop_discovery`
- `lib.rs` — Tauri builder, registers all commands and managed state (`WebRtcTransportState`, `GameEngine`, `UdpState`)
- `config.rs` reads `MATCHBOX_SIGNALING_URL` from `src-tauri/.env` at **compile time** via `build.rs`

### Networking

Two multiplayer modes:
1. **LAN**: mDNS auto-discovery on the same Wi-Fi — zero-config, raw UDP
2. **Online**: WebRTC P2P over the internet via matchbox signaling (`MATCHBOX_SIGNALING_URL`); 4-digit room codes

**Split authority model** — the side that "owns" the puck runs physics authoritatively; the other interpolates via dead reckoning:
- Host owns puck when `puck.y >= TABLE_HEIGHT/2` (host half); client owns it on their half
- Authority switches at the midline with `AUTH_HYSTERESIS` band to prevent rapid flipping
- On authority gain, new owner blends in last-received velocity to preserve momentum

### Tauri IPC

- Frontend → Rust: `invoke(...)` from `@tauri-apps/api/core` (fire-and-forget for pointer input)
- Rust → Frontend (render loop): Tauri `Channel` streams `RenderState` at 60 Hz — **not** Tauri events
- Rust → Frontend (transport events): Tauri events via `listen(...)` from `@tauri-apps/api/event` (e.g. `peer-connected`, `udp-msg-received`)

## Key Conventions

- **Svelte 5 runes only**: `$state`, `$derived`, `$derived.by`, `$props`, `$effect` — never legacy `$:` reactive declarations or stores
- **2D canvas rendering**: `Game.svelte` draws every frame on an HTML `<canvas>` using `CanvasRenderingContext2D`; there is no Three.js/Threlte/WebGL
- **Physics constants live in `config.rs`**: tune gameplay by editing constants there, not inline in `game.rs` or `physics.rs`
- **`src-tauri/.env`** must exist before building (copy from `.env.example`); it sets `MATCHBOX_SIGNALING_URL` at compile time — runtime env vars are ignored
- All Tauri command invocations use `invoke(...)` from `@tauri-apps/api/core`
- Android: `minSdkVersion` 28, app ID `com.dano.puckduel`, signed with `release.keystore`
- `patch_scene.js` and `patch_scene_2.js` are one-off migration scripts — ignore them
