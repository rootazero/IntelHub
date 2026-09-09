#!/usr/bin/env bash
# bootstrap-host.sh — IntelHub host bootstrap (idempotent)
# Run on IntelHub as a user with passwordless sudo:  bash bootstrap-host.sh
# Covers spec §3: packages, Docker (official repo), sshd hardening,
# journald persistence, unattended-upgrades, timesyncd, nftables, directories.
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive
HUB_DIR=/home/zou/IntelHub
LAN=10.10.10.0/24

echo "==> [1/8] apt upgrade + base packages"
sudo -E apt-get update -qq
sudo -E apt-get full-upgrade -y -qq
sudo -E apt-get install -y -qq \
  qemu-guest-agent ca-certificates curl gnupg jq htop \
  unattended-upgrades nftables rsync vim-tiny
sudo systemctl enable --now qemu-guest-agent

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
    ip saddr ${LAN} tcp dport { 22, 7474, 7687, 8080, 8800, 11235, 5001, 3000 } accept
  }
  chain forward {
    type filter hook forward priority 0; policy accept;
    # Docker 29's nftables-managed layout no longer jumps to DOCKER-USER,
    # so container-port gating lives here instead: new inbound connections
    # to published container ports are LAN-only. A drop here is final
    # across all forward-hook base chains; policy stays accept so Docker's
    # own ip/filter chains keep working untouched.
    ct state new ip daddr 172.30.0.0/16 tcp dport { 7474, 7687, 8080, 8800, 11235, 5001, 3000 } ip saddr != ${LAN} counter drop
  }
  chain output {
    type filter hook output priority 0; policy accept;
  }
}
NFT
sudo systemctl enable nftables
sudo nft -f /etc/nftables.conf
echo "    nftables ruleset applied"

echo "==> [3/8] Docker official repository + engine"
if ! command -v docker >/dev/null 2>&1; then
  sudo install -m 0755 -d /etc/apt/keyrings
  curl -fsSL https://download.docker.com/linux/debian/gpg | sudo tee /etc/apt/keyrings/docker.asc >/dev/null
  sudo chmod a+r /etc/apt/keyrings/docker.asc
  echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/debian $(. /etc/os-release && echo "$VERSION_CODENAME") stable" \
    | sudo tee /etc/apt/sources.list.d/docker.list >/dev/null
  sudo -E apt-get update -qq
  sudo -E apt-get install -y -qq \
    docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
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
sudo usermod -aG docker zou

echo "==> [4/8] sshd hardening"
sudo tee /etc/ssh/sshd_config.d/60-intelhub.conf >/dev/null <<SSHD
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin no
PubkeyAuthentication yes
SSHD
sudo sshd -t
sudo systemctl reload ssh

echo "==> [5/8] journald persistent"
sudo mkdir -p /var/log/journal /etc/systemd/journald.conf.d
sudo tee /etc/systemd/journald.conf.d/60-intelhub.conf >/dev/null <<JRNL
[Journal]
Storage=persistent
SystemMaxUse=1G
JRNL
sudo systemctl restart systemd-journald

echo "==> [6/8] unattended-upgrades (security only)"
sudo tee /etc/apt/apt.conf.d/20auto-upgrades >/dev/null <<UU
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
APT::Periodic::AutocleanInterval "7";
UU

echo "==> [7/8] time synchronization"
sudo timedatectl set-ntp true

echo "==> [8/8] IntelHub directory skeleton"
sudo -u zou mkdir -p ${HUB_DIR}/{compose,manifests,config,scripts,backups}

echo
echo "==> bootstrap complete"
docker --version
docker compose version
sudo nft list ruleset | grep -c "chain" | xargs echo "nft chains:"
