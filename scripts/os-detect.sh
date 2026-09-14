#!/usr/bin/env bash
# os-detect.sh — IntelHub OS detection shared helper (single source of truth).
#
# Source this from install.sh / bootstrap-host.sh / update.sh to detect the
# host OS, the package manager family, and the SELinux state. Both Debian
# and RHEL families are supported.
#
# Exports:
#   OS_ID             — distro ID from /etc/os-release (debian, ubuntu, rhel,
#                       centos, rocky, almalinux, fedora, amazon, …)
#   OS_FAMILY         — "deb" | "rpm"
#   OS_PRETTY         — PRETTY_NAME from /etc/os-release (for logs)
#   PKG_INSTALL       — full install command (e.g. "apt-get install -y" or
#                       "dnf install -y")
#   PKG_UPDATE        — refresh repo metadata (apt-get update / dnf check-update)
#   PKG_UPGRADE       — full upgrade (apt-get full-upgrade / dnf upgrade -y)
#   PKG_REPO_DIR      — repo file directory (/etc/apt/sources.list.d or
#                       /etc/yum.repos.d)
#   DOCKER_REPO_FILE  — docker-ce repo file path under PKG_REPO_DIR
#   DOCKER_GPG_PATH   — docker GPG key path (used by both families)
#   SELINUX_STATE     — "enforcing" | "permissive" | "disabled" | "unknown"
#   OS_SUPPORTED      — 1 if OS_FAMILY is one we know how to bootstrap, else 0

set -euo pipefail

# ------------------------------------------------------------ /etc/os-release
[[ -r /etc/os-release ]] || {
  printf 'os-detect: /etc/os-release unreadable — cannot identify OS\n' >&2
  return 1 2>/dev/null || exit 1
}
# shellcheck disable=SC1091
. /etc/os-release

OS_ID="${ID:-unknown}"
OS_PRETTY="${PRETTY_NAME:-$ID}"

# Map distro IDs to a package-manager family. We deliberately accept any
# distro that ID_LIKE marks as debian-ish or rhel-ish (covers derivatives
# without us having to enumerate them all).
case "${ID:-}" in
  # Debian family
  debian|ubuntu|linuxmint|pop|elementary|kali|raspbian) OS_FAMILY="deb" ;;
  # RHEL family (RHEL, CentOS, Rocky, Alma, Fedora, Amazon Linux, Oracle, …)
  rhel|centos|rocky|almalinux|fedora|amazon|ol|almalinuxos) OS_FAMILY="rpm" ;;
  # Anything else: fall back to ID_LIKE
  *)
    case "${ID_LIKE:-}" in
      *debian*) OS_FAMILY="deb" ;;
      *rhel*|*fedora*) OS_FAMILY="rpm" ;;
      *) OS_FAMILY="unknown" ;;
    esac
    ;;
esac

# ---------------------------------------------- package-manager + repo paths
case "$OS_FAMILY" in
  deb)
    PKG_INSTALL="apt-get install -y -qq"
    PKG_UPDATE="apt-get update -qq"
    PKG_UPGRADE="apt-get full-upgrade -y -qq"
    PKG_REPO_DIR="/etc/apt/sources.list.d"
    DOCKER_REPO_FILE="$PKG_REPO_DIR/docker.list"
    DOCKER_GPG_PATH="/etc/apt/keyrings/docker.asc"
    DOCKER_REPO_URL="https://download.docker.com/linux/debian"
    DOCKER_REPO_DIST="${VERSION_CODENAME:-}"
    DOCKER_REPO_ARCH="$(dpkg --print-architecture 2>/dev/null || echo amd64)"
    DOCKER_GPG_URL="https://download.docker.com/linux/debian/gpg"
    ;;
  rpm)
    # dnf is the modern pkg manager (RHEL 8+, Fedora, Rocky 8+/9, Alma 8+/9).
    # yum is symlinked to dnf on those distros, but RHEL 7 / CentOS 7 use
    # the original yum. Use whichever exists.
    if command -v dnf >/dev/null 2>&1; then
      PKG_INSTALL="dnf install -y -q"
      PKG_UPDATE="dnf -q check-update || true"
      PKG_UPGRADE="dnf upgrade -y -q"
    elif command -v yum >/dev/null 2>&1; then
      PKG_INSTALL="yum install -y -q"
      PKG_UPDATE="yum -q check-update || true"
      PKG_UPGRADE="yum update -y -q"
    else
      printf 'os-detect: neither dnf nor yum found on RPM-family OS\n' >&2
      return 1 2>/dev/null || exit 1
    fi
    PKG_REPO_DIR="/etc/yum.repos.d"
    DOCKER_REPO_FILE="$PKG_REPO_DIR/docker-ce.repo"
    DOCKER_GPG_PATH="/etc/pki/rpm-gpg/docker.asc"
    DOCKER_REPO_ARCH="$(uname -m)"
    # Per docker.com docs (https://docs.docker.com/engine/install/), all
    # RHEL-compatible distros (RHEL, CentOS Stream, Rocky, AlmaLinux,
    # Oracle Linux, Amazon Linux 2023) use the /linux/rhel/$VERSION/
    # path. Fedora uses /linux/fedora/$VERSION/. VERSION_ID is a
    # major.minor string like 9.4 or 40.10 — strip the minor.
    RHEL_MAJOR="${VERSION_ID%%.*}"
    case "${ID:-}" in
      fedora)
        DOCKER_REPO_URL="https://download.docker.com/linux/fedora"
        DOCKER_REPO_DIST="${RHEL_MAJOR}"
        ;;
      *)
        # rhel | centos | rocky | almalinux | ol | amzn | amazon
        DOCKER_REPO_URL="https://download.docker.com/linux/rhel"
        DOCKER_REPO_DIST="${RHEL_MAJOR}"
        ;;
    esac
    # GPG key for rpm is published under the same /linux/<distro>/ path.
    DOCKER_GPG_URL="${DOCKER_REPO_URL}/gpg"
    ;;
  *)
    PKG_INSTALL=""; PKG_UPDATE=""; PKG_UPGRADE=""
    PKG_REPO_DIR=""; DOCKER_REPO_FILE=""; DOCKER_GPG_PATH=""
    DOCKER_REPO_URL=""; DOCKER_REPO_DIST=""; DOCKER_REPO_ARCH=""
    DOCKER_GPG_URL=""
    ;;
esac

# ------------------------------------------------ SELinux state (best-effort)
if command -v getenforce >/dev/null 2>&1; then
  SELINUX_STATE="$(getenforce 2>/dev/null || echo unknown)"
else
  SELINUX_STATE="disabled"
fi

# -------------------------------------------- supported? (install.sh consults)
case "$OS_FAMILY" in
  deb|rpm) OS_SUPPORTED=1 ;;
  *)       OS_SUPPORTED=0 ;;
esac

# Export every variable above for any caller that sources us. Disable the
# "unused variable" warnings — these ARE used by sourcing scripts.
# shellcheck disable=SC2034
export OS_ID OS_PRETTY OS_FAMILY PKG_INSTALL PKG_UPDATE PKG_UPGRADE \
       PKG_REPO_DIR DOCKER_REPO_FILE DOCKER_GPG_PATH DOCKER_REPO_URL \
       DOCKER_REPO_DIST DOCKER_REPO_ARCH DOCKER_GPG_URL SELINUX_STATE \
       OS_SUPPORTED