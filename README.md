# PuckDuel 🏒

A fast-paced **P2P air hockey** game for Android — play on the same Wi-Fi or over the internet. First to 6 goals wins.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

## Features

- **LAN multiplayer** — zero-config pairing on the same Wi-Fi via mDNS auto-discovery
- **Online multiplayer** — play over the internet using a 4-digit room code (no relay server — pure P2P WebRTC)
- **Single player** — practice against an AI opponent
- **60 Hz Rust physics engine** — all game logic runs in a native Rust tokio loop
- **Split authority** — fair puck ownership on both sides of the table, no host advantage
- **NAT traversal via STUN** — uses public STUN servers to establish direct peer-to-peer connections
- **Fire-and-forget datagrams** — low-latency WebRTC data channels, no retransmission overhead
- **Haptic feedback** — vibration on hits and goals
- **Countdown** — 3-2-1 before each serve
- **Mute toggle** — silence sound effects mid-game
- **Pause on minimize** — game pauses when you switch apps

## Tech Stack

| Layer | Technology |
|---|---|
| UI | SvelteKit (Svelte 5) + TypeScript |
| Native shell | Tauri 2 |
| Physics engine | Rust (tokio, 60 Hz loop) |
| JS↔Rust IPC | Tauri Channel API (streaming) + `invoke` |
| P2P transport | [matchbox_socket](https://github.com/johanhelsing/matchbox) 0.14 — WebRTC data channels, STUN servers |
| LAN discovery | mDNS-SD (`mdns-sd` crate) |
| Online signaling | matchbox (WebRTC signaling server) |

## Architecture

All physics (collisions, friction, goals, AI, dead reckoning) run in a Rust tokio task at 60 Hz. The JS layer is a pure renderer — it receives `RenderState` frames via the Tauri Channel API and draws them on a `<canvas>`. The only JS→Rust call during gameplay is `set_pointer` (fire-and-forget on each pointer event).

### Networking

Connections are peer-to-peer WebRTC via **matchbox_socket** using STUN servers for NAT traversal. Once connected, game state is exchanged over unreliable WebRTC data channels (channel 0) — there is no relay server in the data path.

**LAN pairing**: both sides call `discover_lan`, which registers an mDNS-SD service and browses simultaneously. The join side sees a live list of discovered hosts and taps to connect.

**Online pairing**: the host calls `host_online`, which creates a WebRTC socket and registers with the matchbox signaling server using a random 4-digit room code. The guest enters the code, connects to the same signaling room, and establishes a direct WebRTC connection.

**Split-authority model**: the player whose half of the table the puck is in owns physics authority. Both players flip authority simultaneously by keying off the peer's last reported puck position, eliminating race conditions at the midline.

## Build

### Prerequisites

- [Rust](https://rustup.rs/)
- [Node.js](https://nodejs.org/)
- [Android SDK + NDK](https://tauri.app/v2/guides/building/android/)
- Tauri CLI: `cargo install tauri-cli`

### Matchbox server setup (online multiplayer only)

1. Deploy the matchbox signaling server (replaces PocketBase):
   ```bash
   docker run -d \
     --name matchbox-server \
     --restart unless-stopped \
     -p 8001:3536 \
     jhelsing/matchbox-server:0.7 \
     matchbox_server
   ```
   The server listens on port 3536 inside the container; map to host port 8001 (or any free port).

2. For production with HTTPS, run behind a reverse proxy (nginx) that upgrades WebSocket connections.

3. Copy the example env file and update with your signaling server URL:
   ```bash
   cp src-tauri/.env.example src-tauri/.env
   # edit src-tauri/.env, set MATCHBOX_SIGNALING_URL (not POCKETBASE_URL)
   ```
   Example:
   ```
   MATCHBOX_SIGNALING_URL=wss://puckduel.dano.win
   ```
   `src-tauri/.env` is gitignored and never committed.

### Android APK / AAB

```bash
npm install
npm run tauri android build          # produces .aab (Play Store)
npm run tauri android build -- --apk # produces .apk (sideload)
```

### Desktop (dev)

```bash
npm install
npm run tauri dev
```

## Privacy Policy

[Privacy Policy](https://skpawar1305.github.io/puck_duel/privacy_policy.html) — PuckDuel collects no personal data. Online signaling only exchanges encrypted connection addresses; no accounts or tracking.

## License

[MIT](LICENSE) © Shubham Pawar
