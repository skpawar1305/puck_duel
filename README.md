# PuckDuel 🏒

A fast-paced **local Wi-Fi multiplayer air hockey** game for Android. Two players on the same network face off in real-time — first to 6 goals wins.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

## Features

- **Local Wi-Fi multiplayer** — play against a friend on the same network
- **Single player** — practice against an AI opponent
- **QR code pairing** — host displays a QR code; guest scans to connect instantly
- **Manual IP entry** — alternative connection if camera isn't available
- **60 Hz Rust physics engine** — all game logic runs in a native Rust tokio loop
- **Split authority** — fair puck ownership on both sides of the table, no host advantage
- **Haptic feedback** — vibration on hits and goals
- **Countdown** — 3-2-1 before each serve
- **Mute toggle** — silence sound effects mid-game
- **Pause on minimize** — game pauses when you switch apps
- **Portrait lock** — consistent layout on all phones

## Tech Stack

| Layer | Technology |
|---|---|
| UI | SvelteKit (Svelte 5) + TypeScript |
| Native shell | Tauri 2 |
| Physics engine | Rust (tokio, 60 Hz loop) |
| JS↔Rust IPC | Tauri Channel API (streaming) + `invoke` |
| Networking | UDP over local Wi-Fi |
| QR scanning | `html5-qrcode` |

## Architecture

All physics (collisions, friction, goals, AI, dead reckoning) run in a Rust tokio task at 60 Hz. The JS layer is a pure renderer — it receives `RenderState` frames via the Tauri Channel API and draws them on a `<canvas>`. The only JS→Rust call is `set_pointer` (fire-and-forget on each pointer event).

Multiplayer uses a **split-authority model**: the player whose half of the table the puck is in owns physics authority. Both players flip authority simultaneously by keying off the peer's last reported puck position, eliminating race conditions at the midline.

## Build

### Prerequisites

- [Rust](https://rustup.rs/)
- [Node.js](https://nodejs.org/)
- [Android SDK + NDK](https://tauri.app/v2/guides/building/android/)
- Tauri CLI: `cargo install tauri-cli`

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

[Privacy Policy](https://skpawar1305.github.io/duelpuck/privacy_policy.html) — PuckDuel collects no personal data.

## License

[MIT](LICENSE) © Shubham Pawar
