# CLAUDE.md

## Commands

```bash
# Full Tauri dev (frontend + Rust backend together)
npm run tauri dev

# Frontend only (Vite dev server at :1420)
npm run dev

# Type-check Svelte/TypeScript
npm run check

# Build frontend only
npm run build

# Build Android AAB (Play Store)
npm run tauri android build

# Build Android APK (sideload)
npm run tauri android build -- --apk

# Rust unit tests (config sanity checks)
cd src-tauri && cargo test
```

## Architecture

PuckDuel is a **client-server** air hockey game for Android/desktop. A game server runs on a Lightsail VPS in Mumbai (`13.232.227.123:9876` UDP). Clients connect to it — no P2P, no NAT issues.

Stack: **Tauri 2** (Rust backend) + **SvelteKit** (Svelte 5, SPA mode) + **TypeScript** + **Tailwind CSS v4** + **Pixi.js 8** (canvas rendering).

### Workspace structure

```
puckduel-core/       # Shared library: physics, GameState, RenderState, config
game-server/         # Standalone binary running on Lightsail (UDP, port 9876)
src-tauri/           # Tauri app (client): connects to game server
frontend-src/        # SvelteKit frontend (routes, components)
```

### Frontend (`frontend-src/`)

- `routes/+page.svelte` — single route; screens: `menu | game | online_join`
- `lib/components/Game.svelte` — Pixi.js canvas renderer; receives `RenderState` at 60Hz via Tauri Channel
- `lib/audio.ts` — Web Audio API synth; call `initAudio()` from a user-gesture handler

### Rust backend (`src-tauri/src/`)

- `game.rs` — Tauri commands: `create_room`, `join_room`, `create_solo`, `start_game`, `stop_game`, etc.
- `config.rs`, `physics.rs` — re-exported from `puckduel-core`

### Game server (`game-server/`)

- UDP server listening on `[::]:9876` (dual-stack IPv4+IPv6)
- Room management with 4-digit codes, 120s timeout, max 256 rooms
- Authoritative physics at 60Hz, sends `RenderState` to both clients via bincode
- AI opponent for solo mode
- Rate limiting: max rooms, one host per IP, input clamping

### Client-server protocol

| Direction | Format | Description |
|-----------|--------|-------------|
| Client → Server | Text `CREATE` | Create a room |
| Server → Client | Text `CREATED:XXXX` | Room code |
| Client → Server | Text `JOIN:XXXX` | Join a room |
| Server → Client | Text `JOINED` | Join accepted |
| Server → Client (both) | Text `START` | Game started |
| Client → Server (60Hz) | Binary: `room_id_len + room_id + paddle_x + paddle_y` | Player input |
| Server → Client (60Hz) | Binary: `b'S' + bincode(RenderState)` | Game state |
| Server → Client | Text `GAME_OVER` | Game ended |

### Tauri IPC

- Frontend → Rust: `invoke(...)` from `@tauri-apps/api/core`
- Rust → Frontend (render loop): Tauri `Channel` streams `RenderState` at 60 Hz

## Key Conventions

- **Svelte 5 runes only**: `$state`, `$derived`, `$props`, `$effect` — no legacy `$:` or stores
- **Pixi.js 8** for canvas rendering
- **Physics constants** live in `puckduel-core/src/config.rs`
- **Keystore** at `src-tauri/keystore.zip` (password-protected), extracted to `release.keystore` + `keystore.properties` for signed builds
- Android `minSdkVersion` is 28; app ID `com.dano.puckduel`
