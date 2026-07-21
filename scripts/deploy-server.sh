#!/usr/bin/env bash
set -euo pipefail
# ──────────────────────────────────────────────────────────────────────────────
# PuckDuel Game Server — Build & Deploy to Lightsail (Mumbai)
# ──────────────────────────────────────────────────────────────────────────────
SSH_KEY="${SSH_KEY:-/home/skpawar1305/Downloads/LightsailDefaultKey-ap-south-1.pem}"
SSH_HOST="${SSH_HOST:-13.232.227.123}"
SSH_USER="${SSH_USER:-ec2-user}"
REMOTE_DIR="${REMOTE_DIR:-/home/ec2-user/puckduel}"

echo "🏗️  Building game-server (release)..."
cargo build --release -p game-server
echo "✅ Build complete"

echo "📦 Copying binary to server..."
ssh -i "$SSH_KEY" "${SSH_USER}@${SSH_HOST}" "mkdir -p ${REMOTE_DIR}"
scp -i "$SSH_KEY" target/release/game-server "${SSH_USER}@${SSH_HOST}:${REMOTE_DIR}/game-server"
scp -i "$SSH_KEY" deploy/game-server.service "${SSH_USER}@${SSH_HOST}:/tmp/game-server.service"

echo "🔧 Installing systemd service..."
ssh -i "$SSH_KEY" "${SSH_USER}@${SSH_HOST}" <<'EOF'
  sudo mv /tmp/game-server.service /etc/systemd/system/game-server.service
  sudo systemctl daemon-reload
  sudo systemctl enable game-server
  sudo systemctl restart game-server
  echo "✅ Service started. Status:"
  sudo systemctl status game-server --no-pager | head -10
EOF

echo "🎉 Deploy complete! Server running on port 8080"
echo "   Test: nc -zu ${SSH_HOST} 8080"
