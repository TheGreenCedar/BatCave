#!/usr/bin/env bash
set -euo pipefail

no_build=0
web_only=0
app_args=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-build)
      no_build=1
      shift
      ;;
    --web-only)
      web_only=1
      shift
      ;;
    --)
      shift
      app_args=("$@")
      break
      ;;
    *)
      app_args+=("$1")
      shift
      ;;
  esac
done

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
app_root="$repo_root/src/BatCave.App"
tauri_dev_script="tauri:dev:linux"

cd "$app_root"

if [[ "$no_build" -eq 0 ]]; then
  npm run build
fi

if [[ "$web_only" -eq 1 ]]; then
  npm run dev
elif [[ "${#app_args[@]}" -gt 0 ]]; then
  npm run "$tauri_dev_script" -- "${app_args[@]}"
else
  npm run "$tauri_dev_script"
fi
