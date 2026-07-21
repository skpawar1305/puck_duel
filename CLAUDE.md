# CLAUDE.md

## Commands

```bash
# Full Tauri dev (frontend + Rust backend together)
bun run tauri dev

# Frontend only (Vite dev server at :1420)
bun run dev

# Type-check Svelte/TypeScript
bun run check

# Build frontend only
bun run build

# Build Android AAB (Play Store)
bun run tauri android build

# Build Android APK (sideload)
bun run tauri android build -- --apk

# Rust unit tests (config sanity checks)
cd src-tauri && cargo test
```

## Architecture

PuckDuel uses a **split-authority** model with **P2P + relay fallback**. The player on whose side the puck resides runs physics locally and sends authoritative `RenderState` to the opponent. A relay server on Lightsail Mumbai (`13.232.227.123:9876` UDP) handles room discovery, NAT hole-punching (PEER address exchange), and fallback forwarding if P2P fails.

Stack: **Tauri 2** (Rust backend) + **SvelteKit** (Svelte 5, SPA mode) + **TypeScript** + **Tailwind CSS v4** + **Pixi.js 8** (canvas rendering).

### Workspace structure

```
puckduel-core/       # Shared library: physics, GameState, RenderState, config
game-server/         # Relay server on Lightsail (UDP, port 9876)
src-tauri/           # Tauri app (client): runs local physics, P2P + relay
frontend-src/        # SvelteKit frontend (routes, components)
```

### Frontend (`frontend-src/`)

- `routes/+page.svelte` — single route; screens: `menu | game | online_join | online_host`
- `lib/components/Game.svelte` — Pixi.js canvas renderer; receives `RenderState` at 60Hz via Tauri Channel
- `lib/audio.ts` — Web Audio API synth; call `initAudio()` from a user-gesture handler

### Rust backend (`src-tauri/src/`)

- `game.rs` — Tauri commands: `create_room`, `join_room`, `create_solo`, `wait_for_opponent`, `start_game`, `stop_game`, etc. Runs split-authority game loop with P2P hole-punching
- `config.rs`, `physics.rs` — re-exported from `puckduel-core`

### Game server (`game-server/`)

- UDP server listening on `[::]:9876` (dual-stack IPv4+IPv6)
- Room management with 4-digit codes, 120s timeout, max 256 rooms
- Sends `PEER:<ip>:<port>` to both players for P2P hole-punching
- Forwards binary packets between players (relay fallback if P2P fails)
- Solo mode: server just sends START, client runs AI locally
- Rate limiting: max rooms, one host per IP

### Protocol

| Direction | Format | Description |
|-----------|--------|-------------|
| Client → Server | Text `CREATE` | Create a room |
| Server → Client | Text `CREATED:XXXX` | Room code |
| Client → Server | Text `JOIN:XXXX` | Join a room |
| Server → Client | Text `JOINED` | Join accepted |
| Server → Client (both) | Text `PEER:<ip>:<port>` | Other player's public address (P2P hole-punch) |
| Server → Client (both) | Text `START` | Game started |
| Client → Peer (60Hz) | Binary: `b'I' + px + py` | Paddle input (via P2P or relay) |
| Client → Peer (60Hz) | Binary: `b'S' + bincode(RenderState)` | Authoritative game state (via P2P or relay) |
| Peer → Client (60Hz) | Binary: `b'I' + px + py` | Opponent paddle + state (from P2P or relay) |
| Client → Server | Text `GAME_OVER` | Game ended |

### P2P hole-punching

1. Relay server sends `PEER` with opponent's public IP:port to both players after JOIN
2. Both sides bind a new UDP socket and send `P2P_HELLO` to each other simultaneously
3. If response received within 2s → P2P established (game data goes direct)
4. If P2P fails (NAT) → relay server forwards all packets as fallback

### Split-authority physics

- Each player runs a local `GameState` at 60Hz
- Player on whose half the puck is becomes authoritative:
  - Runs `server_update(dt, my_paddle, opp_paddle)`
  - Sends completed `RenderState` to opponent via `b'S'` packets
- Non-authoritative player receives opponent's `RenderState` and renders it
- Solo/AI: always authoritative, AI runs locally via simple follow-puck logic

### Tauri IPC

- Frontend → Rust: `invoke(...)` from `@tauri-apps/api/core`
- Rust → Frontend (render loop): Tauri `Channel` streams `RenderState` at 60 Hz

## Key Conventions

- **Svelte 5 runes only**: `$state`, `$derived`, `$props`, `$effect` — no legacy `$:` or stores
- **Pixi.js 8** for canvas rendering; CSP must include `'unsafe-eval'` for WebGL shader compilation
- **Physics constants** live in `puckduel-core/src/config.rs`
- **Paddle velocity** is stored in px/s (pixels per second), divided by `dt` in `server_update`
- **Keystore** at `src-tauri/keystore.zip` (password-protected), extracted to `release.keystore` + `keystore.properties` for signed builds
- Android `minSdkVersion` is 28; app ID `com.dano.puckduel`
