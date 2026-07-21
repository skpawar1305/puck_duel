#!/usr/bin/env bash
set -euo pipefail
# ──────────────────────────────────────────────────────────────────────────────
# PuckDuel VPS Bootstrap — Run on your Mumbai Lightsail instance.
# Installs:
#   1. matchbox_server  – WebRTC signaling (wss://)
#   2. coturn           – TURN relay for CGNAT users
#   3. Caddy or nginx   – TLS termination + reverse proxy
# ──────────────────────────────────────────────────────────────────────────────
DOMAIN="${1:-puckduel.example.com}"          # ← CHANGE ME
TURN_REALM="${DOMAIN}"
TURN_SECRET="$(openssl rand -hex 32)"        # ephemeral credential secret

# ── Prerequisites ─────────────────────────────────────────────────────────────
apt update && apt upgrade -y
apt install -y curl build-essential git tmux ufw
ufw allow 22/tcp
ufw allow 80/tcp
ufw allow 443/tcp
ufw allow 3478/udp          # coturn
ufw allow 49152-65535/udp   # TURN relay ports
ufw --force enable

# ── 1. matchbox_server ────────────────────────────────────────────────────────
if ! command -v matchbox_server &>/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  source "$HOME/.cargo/env"
  cargo install matchbox_server --git https://github.com/johanhelsing/matchbox
fi

# Run via tmux (or replace with a systemd unit)
tmux new-session -d -s matchbox 'matchbox_server --port 8001'

# ── 2. coturn (TURN/STUN) ─────────────────────────────────────────────────────
apt install -y coturn
# Use timestamp-based ephemeral credentials
cat > /etc/turnserver.conf <<TURN
listening-port=3478
tls-listening-port=5349
realm=${TURN_REALM}
fingerprint
lt-cred-mech
use-auth-secret
static-auth-secret=${TURN_SECRET}
total-quota=100
bps-capacity=0
stale-nonce=600
no-multicast-peers
no-cli
TURN

systemctl enable coturn && systemctl restart coturn

# ── 3. Caddy (TLS + reverse proxy for signaling) ──────────────────────────────
curl -fsSL https://caddyserver.com/api/download | bash -s -- -d /usr/bin/caddy

cat > /etc/caddy/Caddyfile <<CADDY
${DOMAIN} {
    reverse_proxy localhost:8001
}
CADDY

systemctl enable caddy && systemctl restart caddy

# ── Print credentials ─────────────────────────────────────────────────────────
echo "═══════════════════════════════════════════════════════"
echo "  MATCHBOX_SIGNALING_URL=wss://${DOMAIN}"
echo "  TURN_SERVER_URL=turn:${DOMAIN}:3478"
echo "  TURN_SERVER_USERNAME=any-user"
echo "  TURN_SERVER_PASSWORD=${TURN_SECRET}"
echo "═══════════════════════════════════════════════════════"
echo "Copy these into src-tauri/.env before building."
