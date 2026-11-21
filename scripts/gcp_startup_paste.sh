#!/bin/bash
# ===== FILL IN THESE THREE LINES, THEN PASTE THE WHOLE SCRIPT INTO "Startup script" WHEN CREATING THE VM =====
HUB="PASTE_HUB_ADDRESS"               # the hub's Tailscale address and port, like 100.x.y.z:7300
SCREENER_TOKEN="PASTE_SCREENER_TOKEN"    # contents of ~/quip-remote/screener-token on the hub machine
TS_AUTHKEY="PASTE_TAILSCALE_KEY"         # tskey-auth-... from the Tailscale admin page (1 day, ephemeral)
# ===== nothing below needs editing =====
# Google Compute Engine startup script: turns a CPU-only VM into a quip-cpu-screener.
# Runs as root on every boot and is safe to run again.
#
# Values are read from instance metadata (Compute Engine > VM > Edit > Custom metadata), or from environment variables:
#   hub-address     host:port of the broker as seen from the VM (its Tailscale address), e.g. 100.x.y.z:7300     (HUB)
#   screener-token  the shared screener token                                                                    (SCREENER_TOKEN)
#   ts-authkey      a Tailscale auth key, ideally one-off + ephemeral + pre-approved (only needed on first boot)  (TS_AUTHKEY)
#   screener-bin-url (optional) URL of a prebuilt `screener` binary; if absent the repo is built from source      (SCREENER_BIN_URL)
#   repo-url         (optional) git URL to build from, default below                                             (REPO_URL)
# Note: custom metadata is readable by anyone with access to the project; use a short-lived Tailscale key.
set -euo pipefail
exec > >(tee -a /var/log/quip-screener-setup.log) 2>&1
echo "== quip-screener setup $(date -u +%FT%TZ)"

meta() { curl -fs -H 'Metadata-Flavor: Google' "http://metadata.google.internal/computeMetadata/v1/instance/attributes/$1" 2>/dev/null || true; }
HUB="${HUB:-$(meta hub-address)}"
SCREENER_TOKEN="${SCREENER_TOKEN:-$(meta screener-token)}"
TS_AUTHKEY="${TS_AUTHKEY:-$(meta ts-authkey)}"
SCREENER_BIN_URL="${SCREENER_BIN_URL:-$(meta screener-bin-url)}"
REPO_URL="${REPO_URL:-$(meta repo-url)}"; REPO_URL="${REPO_URL:-https://github.com/rootztigmod/quip-cpu-screener.git}"
NAME="${NAME:-$(hostname)}"
[ -n "$HUB" ] || { echo "missing hub-address"; exit 1; }
[ -n "$SCREENER_TOKEN" ] || { echo "missing screener-token"; exit 1; }

# 1. Tailscale (joins the private network the hub is on)
if ! command -v tailscale >/dev/null; then
  curl -fsSL https://tailscale.com/install.sh | sh
fi
if ! tailscale ip -4 >/dev/null 2>&1; then
  [ -n "$TS_AUTHKEY" ] || { echo "not on Tailscale and no ts-authkey given"; exit 1; }
  tailscale up --authkey="$TS_AUTHKEY" --hostname="$NAME" --accept-dns=false
fi
echo "tailscale address: $(tailscale ip -4 | head -1)"

# 2. the screener binary
install -d /opt/quip-screener
if [ -n "$SCREENER_BIN_URL" ]; then
  curl -fsSL "$SCREENER_BIN_URL" -o /opt/quip-screener/screener.new && chmod +x /opt/quip-screener/screener.new && mv /opt/quip-screener/screener.new /opt/quip-screener/screener
elif [ ! -x /opt/quip-screener/screener ]; then
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -y && apt-get install -y build-essential curl git pkg-config
  if ! command -v cargo >/dev/null && [ ! -x /root/.cargo/bin/cargo ]; then curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal; fi
  export PATH="/root/.cargo/bin:$PATH"
  rm -rf /opt/quip-screener/src && git clone --depth 1 "$REPO_URL" /opt/quip-screener/src
  (cd /opt/quip-screener/src && RUSTFLAGS="-C target-cpu=native" cargo build --release --locked --bin screener)
  install -m 755 /opt/quip-screener/src/target/release/screener /opt/quip-screener/screener
fi

# 3. service (the token lives in a root-only file, not on the command line)
umask 077
printf 'QUIP_SCREENER_TOKEN=%s\n' "$SCREENER_TOKEN" > /etc/quip-screener.env
THREADS="$(nproc)"
cat > /etc/systemd/system/quip-screener.service <<UNIT
[Unit]
Description=Quip CPU screener
After=network-online.target tailscaled.service
Wants=network-online.target

[Service]
EnvironmentFile=/etc/quip-screener.env
ExecStart=/opt/quip-screener/screener --hub ${HUB} --threads ${THREADS} --name ${NAME}
Restart=always
RestartSec=5
Nice=5

[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload
systemctl enable quip-screener >/dev/null 2>&1
systemctl restart quip-screener
sleep 5
systemctl --no-pager --lines=5 status quip-screener || true
echo "== done"
