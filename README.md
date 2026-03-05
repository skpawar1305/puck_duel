# PuckDuel 🏒

A fast-paced **P2P air hockey** game for Android — play on the same Wi-Fi or over the internet. First to 6 goals wins.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

## Features

- **LAN multiplayer** — zero-config pairing on the same Wi-Fi via mDNS auto-discovery
- **Online multiplayer** — play over the internet using a 6-character room code (no relay server — pure P2P QUIC)
- **Single player** — practice against an AI opponent
- **60 Hz Rust physics engine** — all game logic runs in a native Rust tokio loop
- **Split authority** — fair puck ownership on both sides of the table, no host advantage
- **IPv6-first networking** — prefers IPv6, falls back to IPv4; relay servers never used
- **Fire-and-forget datagrams** — low-latency QUIC datagrams, no retransmission overhead
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
| P2P transport | [iroh](https://github.com/n0-computer/iroh) 0.96 — QUIC, `RelayMode::Disabled` |
| LAN discovery | mDNS-SD (`mdns-sd` crate) |
| Online signaling | Supabase (REST, room code exchange only) |

## Architecture

All physics (collisions, friction, goals, AI, dead reckoning) run in a Rust tokio task at 60 Hz. The JS layer is a pure renderer — it receives `RenderState` frames via the Tauri Channel API and draws them on a `<canvas>`. The only JS→Rust call during gameplay is `set_pointer` (fire-and-forget on each pointer event).

### Networking

Connections are pure peer-to-peer QUIC via **iroh** with relay servers completely disabled (`RelayMode::Disabled`). Once connected, game state is exchanged as fire-and-forget QUIC datagrams — there is no TCP fallback or relay path.

**LAN pairing**: both sides call `discover_lan`, which registers an mDNS-SD service and browses simultaneously. The join side sees a live list of discovered hosts and taps to connect.

**Online pairing**: the host calls `host_online`, which posts its iroh `EndpointAddr` (serialized JSON) to a Supabase table keyed by a random 6-char room code. The guest enters the code, fetches the `EndpointAddr`, and dials directly.

**Split-authority model**: the player whose half of the table the puck is in owns physics authority. Both players flip authority simultaneously by keying off the peer's last reported puck position, eliminating race conditions at the midline.

## Build

### Prerequisites

- [Rust](https://rustup.rs/)
- [Node.js](https://nodejs.org/)
- [Android SDK + NDK](https://tauri.app/v2/guides/building/android/)
- Tauri CLI: `cargo install tauri-cli`

### Supabase setup (online multiplayer only)

1. Create a free project at [supabase.com](https://supabase.com).
2. Run in the SQL editor:
   ```sql
   create table rooms (
     code text primary key,
     node_addr text not null,
     created_at timestamptz default now()
   );
   alter table rooms enable row level security;
   create policy "public read/write" on rooms for all using (true) with check (true);
   ```
3. Copy the example env file and fill in your credentials:
   ```bash
   cp src-tauri/.env.example src-tauri/.env
   # then edit src-tauri/.env with your Project URL and anon key
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
