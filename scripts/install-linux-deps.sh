#!/usr/bin/env bash
set -euo pipefail

with_bpftrace=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --with-bpftrace)
      with_bpftrace=1
      shift
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if ! command -v apt-get >/dev/null 2>&1; then
  echo "install-linux-deps.sh currently supports Debian/Ubuntu hosts with apt-get." >&2
  exit 2
fi

sudo_cmd=()
if [[ "$(id -u)" -ne 0 ]]; then
  sudo_cmd=(sudo)
fi

packages=(
  binutils \
  build-essential \
  curl \
  wget \
  file \
  python3 \
  pkg-config \
  libssl-dev \
  libgtk-3-dev \
  libwebkit2gtk-4.1-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libxdo-dev \
  squashfs-tools
)
if [[ "$with_bpftrace" -eq 1 ]]; then
  packages+=(bpftrace)
fi

"${sudo_cmd[@]}" apt-get update
"${sudo_cmd[@]}" apt-get install -y "${packages[@]}"

if command -v rustup >/dev/null 2>&1; then
  rustup component add rustfmt
else
  echo "rustup was not found; install a stable Rust toolchain with rustfmt before running scripts/validate-tauri.sh." >&2
fi
