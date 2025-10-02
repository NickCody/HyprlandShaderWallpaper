#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Lambda Shade installer

Usage: install.sh [options]

Options:
  --prefix <path>        Cargo install prefix (passed to `cargo install --root`). Optional.
  --share-dir <path>     Directory holding bundled shader defaults. Defaults to
                         "${XDG_DATA_HOME:-$HOME/.local/share}/lambdash" in user mode.
  --system               Install for all users (prefix=/usr/local, share-dir=/usr/share/lambdash).
                         Requires root privileges.
  --ref <git-ref>        Git branch, tag, or commit to install from (default: main).
  --repo <git-url>       Source repository URL (default: https://github.com/NickCody/HyprlandShaderWallpaper.git).
  --source <path>        Use an existing local checkout instead of cloning.
  --no-sync              Skip running `lambdash defaults sync` after installation.
  --skip-build           Skip `cargo install` (useful if the binary is already present).
  --offline              Pass `--offline` to cargo when building.
  --help                 Show this help message and exit.

Examples:
  # User-mode install (default behaviour)
  bash install.sh

  # System-wide install
  sudo bash install.sh --system

  # One-liner from GitHub
  bash -c "$(curl -fsSL https://raw.githubusercontent.com/NickCody/HyprlandShaderWallpaper/main/scripts/install.sh)" -- --prefix "$HOME/.local"
USAGE
}

ensure_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "[lambdash-installer] Required command not found: $1" >&2
    exit 1
  fi
}

clean_up() {
  if [[ -n "${TMPDIR_ROOT:-}" && -d "${TMPDIR_ROOT}" ]]; then
    rm -rf "${TMPDIR_ROOT}"
  fi
}

trap clean_up EXIT

prefix=""
share_dir=""
system_install=0
repo_url="https://github.com/NickCody/HyprlandShaderWallpaper.git"
ref="main"
source_dir=""
runs_sync=1
skip_build=0
cargo_offline=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix)
      prefix="$2"
      shift 2
      ;;
    --share-dir)
      share_dir="$2"
      shift 2
      ;;
    --system)
      system_install=1
      prefix="/usr/local"
      share_dir="/usr/share/lambdash"
      shift
      ;;
    --ref)
      ref="$2"
      shift 2
      ;;
    --repo)
      repo_url="$2"
      shift 2
      ;;
    --source)
      source_dir="$2"
      shift 2
      ;;
    --no-sync)
      runs_sync=0
      shift
      ;;
    --skip-build)
      skip_build=1
      shift
      ;;
    --offline)
      cargo_offline=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    *)
      echo "[lambdash-installer] Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ $system_install -eq 1 && $(id -u) -ne 0 ]]; then
  echo "[lambdash-installer] --system requires root privileges." >&2
  exit 1
fi

if [[ -z "$share_dir" ]]; then
  share_dir="${XDG_DATA_HOME:-$HOME/.local/share}/lambdash"
fi

ensure_command cargo
ensure_command git
ensure_command tar

TMPDIR_ROOT=$(mktemp -d 2>/dev/null || mktemp -d -t lambdash-install)
repo_path="${TMPDIR_ROOT}/LambdaShade"

if [[ -n "$source_dir" ]]; then
  echo "[lambdash-installer] Using local source: $source_dir"
  mkdir -p "$repo_path"
  (cd "$source_dir" && git rev-parse HEAD >/dev/null 2>&1) || {
    echo "[lambdash-installer] Source directory must be a git checkout." >&2
    exit 1
  }
  tar -C "$source_dir" --exclude='.git' -cf - . | tar -C "$repo_path" -xf -
else
  echo "[lambdash-installer] Cloning $repo_url@$ref"
  git clone --depth 1 --branch "$ref" "$repo_url" "$repo_path" >/dev/null
fi

if [[ $skip_build -eq 0 ]]; then
  cargo_args=(install --path "$repo_path/crates/lambdash" --locked --force)
  if [[ -n "$prefix" ]]; then
    cargo_args+=(--root "$prefix")
  fi
  if [[ $cargo_offline -eq 1 ]]; then
    cargo_args+=(--offline)
  fi

  echo "[lambdash-installer] Building lambdash via cargo"
  cargo "${cargo_args[@]}"
else
  echo "[lambdash-installer] Skipping cargo build (--skip-build)"
fi

echo "[lambdash-installer] Installing bundled shader defaults into $share_dir"
mkdir -p "$share_dir"
for subdir in local-shaders multi; do
  if [[ -d "$repo_path/$subdir" ]]; then
    rm -rf "$share_dir/$subdir"
    mkdir -p "$share_dir/$subdir"
    tar -C "$repo_path/$subdir" -cf - . | tar -C "$share_dir/$subdir" -xf -
  fi
done

if [[ -f "$repo_path/VERSION" ]]; then
  cp "$repo_path/VERSION" "$share_dir/VERSION"
else
  printf '%s\n' "$ref" >"$share_dir/VERSION"
fi

if [[ $runs_sync -eq 1 ]]; then
  echo "[lambdash-installer] Syncing defaults into user directories"
  LAMBDASH_SHARE_DIR="$share_dir" lambdash defaults sync || {
    echo "[lambdash-installer] Warning: defaults sync failed" >&2
  }
fi

bin_path=$(command -v lambdash 2>/dev/null || true)
if [[ -z "$bin_path" && -n "$prefix" ]]; then
  bin_path="$prefix/bin/lambdash (add to PATH)"
fi

echo "[lambdash-installer] lambdash installation complete"
if [[ -n "$bin_path" ]]; then
  echo "[lambdash-installer] Binary location: $bin_path"
else
  echo "[lambdash-installer] Binary installed; ensure your cargo bin directory is on PATH."
fi
echo "[lambdash-installer] Share directory: $share_dir"

echo "[lambdash-installer] Run 'lambdash defaults where' to verify configuration."
