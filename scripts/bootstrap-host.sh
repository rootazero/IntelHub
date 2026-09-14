#!/usr/bin/env bash
# bootstrap-host.sh — IntelHub host bootstrap (idempotent)
# Run on IntelHub as a user with passwordless sudo:  bash bootstrap-host.sh
# Covers spec §3: packages, Docker (official repo), sshd hardening,
# journald persistence, unattended-upgrades, timesyncd, nftables, directories.
#
# Supports Debian (deb) and RHEL (rpm) families. The package manager and
# per-distro paths are resolved by os-detect.sh — single source of truth
# shared with install.sh. SELinux (rpm family): if enforcing, the script
# downgrades to permissive for the install and prints a clear warning;
# production hardening with per-mount :z labels is tracked separately.

set -euo pipefail

HUB_DIR="${INTELHUB_HOME:-/home/zou/IntelHub}"
LAN="${INTELHUB_LAN:-10.10.10.0/24}"
RUN_USER="${INTELHUB_USER:-$(id -un)}"

# shellcheck disable=SC1091
. "$(dirname "${BASH_SOURCE[0]}")/os-detect.sh"

[[ "$OS_SUPPORTED" == "1" ]] || {
  echo "bootstrap-host.sh: unsupported OS family '${OS_FAMILY:-?}' (ID='${OS_ID:-?}')" >&2
  echo "Supported: deb (debian, ubuntu, …) and rpm (rhel, rocky, almalinux, fedora, …)" >&2
  exit 1
}

# Suppress interactive prompts on deb-family (no-op on rpm-family).
export DEBIAN_FRONTEND="${DEBIAN_FRONTEND:-noninteractive}"

# ----------------- per-family package-name mapping -----------------
# Names that differ between deb and rpm are aliased here so the body of
# the script stays generic. Add new families (e.g. suse) by extending
# os-detect.sh + this table.
case "$OS_FAMILY" in
  deb)
    PKG_UNATTENDED="unattended-upgrades"
    PKG_AUTO_CFG_DIR="/etc/apt/apt.conf.d"
    PKG_AUTO_CFG_FILE="$PKG_AUTO_CFG_DIR/20auto-upgrades"
    ;;
  rpm)
    PKG_UNATTENDED="dnf-automatic"
    PKG_AUTO_CFG_DIR="/etc/dnf"
    PKG_AUTO_CFG_FILE="$PKG_AUTO_CFG_DIR/automatic.conf"
    ;;
esac

# Common to both families (identical package names on deb and rpm):
# qemu-guest-agent, ca-certificates, curl, gnupg, jq, htop, nftables,
# rsync, vim-minimal/vim-tiny, openssh-server (already installed in
# cloud images; we just re-enable + harden).
PKG_BASE=(qemu-guest-agent ca-certificates curl gnupg jq htop nftables rsync)

if [[ "$OS_FAMILY" == "deb" ]]; then
  PKG_BASE+=(vim-tiny)
elif [[ "$OS_FAMILY" == "rpm" ]]; then
  PKG_BASE+=(vim-minimal openssh-server)
fi

# ============================================================ 1/8 packages
echo "==> [1/8] base packages ($OS_FAMILY)"
sudo -E $PKG_UPDATE
sudo -E $PKG_UPGRADE
sudo -E $PKG_INSTALL "${PKG_BASE[@]}"

# Service names diverge for qemu-guest-agent on some distros; assume the
# systemd unit name is qemu-guest-agent (true on debian + most rpm).
sudo systemctl enable --now qemu-guest-agent || true

# ============================================================ 2/8 nftables
echo "==> [2/8] nftables (installed BEFORE docker so chain merge order is deterministic)"
sudo tee /etc/nftables.conf >/dev/null <<NFT
#!/usr/sbin/nft -f
flush ruleset

table inet filter {
  chain input {
    type filter hook input priority 0; policy drop;
    iifname "lo" accept
    ct state established,related accept
    ct state invalid drop
    ip protocol icmp accept
    ip6 nexthdr icmpv6 accept
    ip saddr ${LAN} tcp dport { 22, 8080, 8800, 11235, 5001, 3000 } accept
  }
  chain forward {
    type filter hook forward priority 0; policy accept;
    # Docker 29's nftables-managed layout no longer jumps to DOCKER-USER,
    # so container-port gating lives here instead: new inbound connections
    # to published container ports are LAN-only. A drop here is final
    # across all forward-hook base chains; policy stays accept so Docker's
    # own ip/filter chains keep working untouched.
    ct state new ip daddr 172.30.0.0/16 tcp dport { 8080, 8800, 11235, 5001, 3000 } ip saddr != ${LAN} counter drop
  }
  chain output {
    type filter hook output priority 0; policy accept;
  }
}
NFT
sudo systemctl enable nftables
sudo nft -f /etc/nftables.conf
echo "    nftables ruleset applied"

# ============================================================ 3/8 Docker
echo "==> [3/8] Docker official repository + engine"
if ! command -v docker >/dev/null 2>&1; then
  if [[ "$OS_FAMILY" == "deb" ]]; then
    sudo install -m 0755 -d /etc/apt/keyrings
    curl -fsSL https://download.docker.com/linux/debian/gpg | sudo tee "$DOCKER_GPG_PATH" >/dev/null
    sudo chmod a+r "$DOCKER_GPG_PATH"
    echo "deb [arch=$(dpkg --print-architecture) signed-by=$DOCKER_GPG_PATH] https://download.docker.com/linux/debian $(. /etc/os-release && echo "$VERSION_CODENAME") stable" \
      | sudo tee "$DOCKER_REPO_FILE" >/dev/null
    sudo -E $PKG_UPDATE
    sudo -E $PKG_INSTALL docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
  else
    # rpm family: docker.com's repo file keyed on $releasever.
    curl -fsSL https://download.docker.com/linux/centos/gpg | sudo tee "$DOCKER_GPG_PATH" >/dev/null
    sudo chmod a+r "$DOCKER_GPG_PATH"
    cat <<EOF | sudo tee "$DOCKER_REPO_FILE" >/dev/null
[docker-ce-stable]
name=Docker CE Stable - \$basearch
baseurl=https://download.docker.com/linux/centos/\$releasever/\$basearch/stable
enabled=1
gpgcheck=1
gpgkey=$DOCKER_GPG_PATH
EOF
    sudo -E $PKG_UPDATE
    sudo -E $PKG_INSTALL docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
  fi
else
  echo "    docker already installed: $(docker --version)"
fi

sudo tee /etc/docker/daemon.json >/dev/null <<JSON
{
  "log-driver": "json-file",
  "log-opts": { "max-size": "10m", "max-file": "3" },
  "default-address-pools": [ { "base": "172.30.0.0/16", "size": 24 } ],
  "live-restore": true
}
JSON
sudo systemctl restart docker
sudo systemctl enable docker containerd
sudo usermod -aG docker "$RUN_USER"

# ============================================================ 4/8 sshd
echo "==> [4/8] sshd hardening"
sudo tee /etc/ssh/sshd_config.d/60-intelhub.conf >/dev/null <<SSHD
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin no
PubkeyAuthentication yes
SSHD
sudo sshd -t
sudo systemctl reload ssh

# ============================================================ 5/8 journald
echo "==> [5/8] journald persistent"
sudo mkdir -p /var/log/journal /etc/systemd/journald.conf.d
sudo tee /etc/systemd/journald.conf.d/60-intelhub.conf >/dev/null <<JRNL
[Journal]
Storage=persistent
SystemMaxUse=1G
JRNL
sudo systemctl restart systemd-journald

# ====================================================== 6/8 unattended-upgrades
echo "==> [6/8] unattended-upgrades ($PKG_UNATTENDED)"
sudo -E $PKG_INSTALL "$PKG_UNATTENDED"
if [[ "$OS_FAMILY" == "deb" ]]; then
  sudo tee "$PKG_AUTO_CFG_FILE" >/dev/null <<UU
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
APT::Periodic::AutocleanInterval "7";
UU
  sudo systemctl enable --now unattended-upgrades
else
  # rpm: enable dnf-automatic timer (default config is fine for security-only).
  sudo systemctl enable --now dnf-automatic.timer
fi

# ============================================================ 7/8 timesyncd
echo "==> [7/8] time synchronization"
sudo timedatectl set-ntp true

# ============================================================ 8/8 dirs
echo "==> [8/8] IntelHub directory skeleton"
sudo -u "$RUN_USER" mkdir -p "${HUB_DIR}"/{compose,manifests,config,scripts,backups}

# ==================================================== SELinux (rpm-family only)
if [[ "$OS_FAMILY" == "rpm" && "$SELINUX_STATE" == "Enforcing" ]]; then
  cat <<WARN >&2

!! ============================================================
!! SELinux is ENFORCING. IntelHub's compose stack bind-mounts
!! $HUB_DIR (and subdirs) into postgres/redis/qdrant/neo4j etc.
!! Without per-volume :z labels (TODO), the daemon processes
!! can't read those mounts and the stack will fail to start.
!!
!! This bootstrap downgrades SELinux to PERMISSIVE for the
!! install. To re-enable after adding :z labels:
!!   sudo setenforce 1
!! And persist via /etc/selinux/config after reboot.
!! ============================================================
WARN
  sudo setenforce Permissive || true
fi

echo
echo "==> bootstrap complete"
docker --version
docker compose version
sudo nft list ruleset | grep -c "chain" | xargs echo "nft chains:"