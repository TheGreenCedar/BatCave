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

"${sudo_cmd[@]}" apt-get update
if [[ "$with_bpftrace" -eq 1 ]]; then
  minimum_bpftrace_version="0.22.0"
  installed_bpftrace_version=""
  if command -v bpftrace >/dev/null 2>&1; then
    installed_bpftrace_version="$(bpftrace --version 2>&1 | sed -nE 's/.*v([0-9]+\.[0-9]+\.[0-9]+).*/\1/p' | head -n 1)"
  fi

  if [[ -z "$installed_bpftrace_version" ]] || ! dpkg --compare-versions "$installed_bpftrace_version" ge "$minimum_bpftrace_version"; then
    candidate_bpftrace_version="$(apt-cache policy bpftrace | sed -nE 's/^  Candidate: (.+)$/\1/p' | head -n 1)"
    if [[ -z "$candidate_bpftrace_version" ]] || [[ "$candidate_bpftrace_version" == "(none)" ]] || ! dpkg --compare-versions "$candidate_bpftrace_version" ge "$minimum_bpftrace_version"; then
      echo "Linux process-network attribution requires bpftrace $minimum_bpftrace_version or newer." >&2
      if [[ -n "$candidate_bpftrace_version" ]]; then
        echo "The configured apt repositories offer bpftrace $candidate_bpftrace_version, so --with-bpftrace will not install an unsupported package." >&2
      else
        echo "The configured apt repositories do not offer a bpftrace package." >&2
      fi
      echo "Install a newer bpftrace package or trusted upstream build, then rerun this script with --with-bpftrace to verify it." >&2
      exit 1
    fi
    packages+=(bpftrace)
  fi
fi
"${sudo_cmd[@]}" apt-get install -y "${packages[@]}"

if [[ "$with_bpftrace" -eq 1 ]]; then
  installed_bpftrace_version="$(bpftrace --version 2>&1 | sed -nE 's/.*v([0-9]+\.[0-9]+\.[0-9]+).*/\1/p' | head -n 1)"
  if [[ -z "$installed_bpftrace_version" ]]; then
    echo "Could not determine the installed bpftrace version; BatCave requires $minimum_bpftrace_version or newer." >&2
    exit 1
  fi
  if ! dpkg --compare-versions "$installed_bpftrace_version" ge "$minimum_bpftrace_version"; then
    echo "The distribution installed bpftrace $installed_bpftrace_version, but BatCave requires $minimum_bpftrace_version or newer." >&2
    echo "Install a newer bpftrace package or upstream build before enabling Linux process-network attribution." >&2
    exit 1
  fi
fi

if command -v rustup >/dev/null 2>&1; then
  rustup component add rustfmt
else
  echo "rustup was not found; install a stable Rust toolchain with rustfmt before running scripts/validate-tauri.sh." >&2
fi
