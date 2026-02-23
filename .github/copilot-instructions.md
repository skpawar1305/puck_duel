# Copilot Instructions

## Commands

```bash
# Full Tauri dev (frontend + Rust backend together)
npm run tauri dev

# Frontend only (Vite)
npm run dev

# Type-check Svelte/TS
npm run check
npm run check:watch

# Build frontend
npm run build

# Bundle Tauri app
npm run tauri build
```

There is no test suite.

## Architecture

This is a **local-network multiplayer Air Hockey game** built with:
- **Tauri 2** (Rust backend) + **SvelteKit** (Svelte 5, SPA mode) + **TypeScript**
- **Threlte** (`@threlte/core`, `@threlte/extras`, `@threlte/rapier`) — Svelte wrappers for Three.js and Rapier3D physics
- **Tailwind CSS v4**

### Frontend layer

- Single SvelteKit route: `src/routes/+page.svelte` — manages screens (`menu | host | join | game`) and invokes Tauri commands for networking
- `src/lib/components/Game.svelte` — thin wrapper that sets up `<Canvas>` (Threlte) and `<World>` (Rapier)
- `src/lib/components/Scene.svelte` — all game logic: 3D scene, physics bodies, game loop (`useTask()`), pointer input, network sync, scoring
- `src/lib/audio.ts` — Web Audio API synth; must call `initAudio()` from a user-gesture handler before playing sounds
- `src/lib/components/QRScanner.svelte` — wraps `html5-qrcode` for scanning host IP QR codes

SvelteKit is configured with `adapter-static` in SPA mode (`fallback: "index.html"`) — required for Tauri (no SSR).

### Rust backend (`src-tauri/src/`)

- `udp_server.rs` — exposes five Tauri commands: `start_udp_host`, `host_send_msg`, `connect_udp_client`, `client_send_msg`, `get_local_ips`
- Host binds UDP on port **8080** (`0.0.0.0:8080`); client binds an ephemeral port and sends to `<host_ip>:8080`
- Incoming UDP packets are forwarded to Svelte as Tauri events named **`udp-msg-received`** with payload `[senderAddr, jsonString]`

### Networking protocol

All messages are JSON. Two message types:

| Sender | Type | Fields |
|--------|------|--------|
| Client → Host | `"input"` | `pos: [x, z]`, optional `puck: [x, z]`, `vel: [x, z]`, `score` |
| Host → Client | `"state"` | `hostPaddle: [x, z]`, optional `puck: [x, z]`, `vel: [x, z]`, `score` |

### Authoritative physics model

The side that currently "owns" the puck runs physics authoritatively; the other side interpolates.

- **Host** owns the puck when `puck.z >= 0` (host half of table)
- **Client** owns the puck when `puck.z < 0` (client half)
- In single-player mode, host is always authoritative

The `amIAuthoritative` flag is `$derived` from `puckZ`. When authority changes, the new owner adopts the last-received velocity (`targetPuckVel`) to keep momentum continuous.

## Key Conventions

- **Svelte 5 runes only**: use `$state`, `$derived`, `$derived.by`, `$props`, `$effect` — not old `$:` reactive declarations or stores
- **Threlte `useTask()`** is the per-frame game loop; all physics updates and network broadcasts happen there
- **Positions are `[x, y, z]` tuples** for Svelte state but plain objects `{x, y, z}` for Rapier APIs. Network messages send only `[x, z]` (y is fixed: `0.2` for paddles, `0.1` for puck)
- All Tauri command invocations use `invoke(...)` from `@tauri-apps/api/core`; all event subscriptions use `listen(...)` from `@tauri-apps/api/event`
- `patch_scene.js` and `patch_scene_2.js` are one-off migration scripts — not part of the build and can be ignored
- `release.keystore` is used for Android signing (Tauri mobile target)
- Camera height adapts to viewport aspect ratio in `Scene.svelte` via `cameraY` derived state
