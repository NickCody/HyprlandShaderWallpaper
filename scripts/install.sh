#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
WallShader installer

Usage: install.sh [options]

Options:
  --prefix <path>        Cargo install prefix (passed to `cargo install --root`). Optional.
  --data-dir <path>      Destination for bundled shader packs (default: "${XDG_DATA_HOME:-$HOME/.local/share}/wallshader").
  --system               Install for all users (prefix=/usr/local, data-dir=/usr/share/wallshader).
                         Requires root privileges.
  --ref <git-ref>        Git branch, tag, or commit to install from (default: main).
  --repo <git-url>       Source repository URL (default: https://github.com/NickCody/WallShader.git).
  --source <path>        Use an existing local checkout instead of cloning.
  --skip-build           Skip `cargo install` (useful if the binary is already present).
  --offline              Pass `--offline` to cargo when building.
  --help                 Show this help message and exit.

Examples:
  # User-mode install (default behaviour)
  bash install.sh

  # System-wide install
  sudo bash install.sh --system

  # One-liner from GitHub
  bash -c "$(curl -fsSL https://raw.githubusercontent.com/NickCody/WallShader/main/scripts/install.sh)" -- --prefix "$HOME/.local"
USAGE
}

ensure_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "[wallshader-installer] Required command not found: $1" >&2
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
data_dir=""
system_install=0
repo_url="https://github.com/NickCody/WallShader.git"
ref="main"
source_dir=""
skip_build=0
cargo_offline=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix)
      prefix="$2"
      shift 2
      ;;
    --data-dir|--share-dir)
      data_dir="$2"
      shift 2
      ;;
    --system)
      system_install=1
      prefix="/usr/local"
      data_dir="/usr/share/wallshader"
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
      echo "[wallshader-installer] Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ $system_install -eq 1 && $(id -u) -ne 0 ]]; then
  echo "[wallshader-installer] --system requires root privileges." >&2
  exit 1
fi

if [[ -z "$data_dir" ]]; then
  data_dir="${XDG_DATA_HOME:-$HOME/.local/share}/wallshader"
fi

ensure_command cargo
ensure_command git
ensure_command tar
ensure_command install

TMPDIR_ROOT=$(mktemp -d 2>/dev/null || mktemp -d -t wallshader-install)
repo_path="${TMPDIR_ROOT}/WallShaderade"

if [[ -n "$source_dir" ]]; then
  echo "[wallshader-installer] Using local source: $source_dir"
  mkdir -p "$repo_path"
  (cd "$source_dir" && git rev-parse HEAD >/dev/null 2>&1) || {
    echo "[wallshader-installer] Source directory must be a git checkout." >&2
    exit 1
  }
  tar -C "$source_dir" --exclude='.git' -cf - . | tar -C "$repo_path" -xf -
else
  echo "[wallshader-installer] Cloning $repo_url@$ref"
  git clone --depth 1 --branch "$ref" "$repo_url" "$repo_path" >/dev/null
fi

if [[ $skip_build -eq 0 ]]; then
  cargo_args=(install --path "$repo_path/crates/wallshader" --locked --force)
  if [[ -n "$prefix" ]]; then
    cargo_args+=(--root "$prefix")
  fi
  if [[ $cargo_offline -eq 1 ]]; then
    cargo_args+=(--offline)
  fi

  echo "[wallshader-installer] Building wallshader via cargo"
  cargo "${cargo_args[@]}"
else
  echo "[wallshader-installer] Skipping cargo build (--skip-build)"
fi

echo "[wallshader-installer] Installing bundled shaders and playlists into $data_dir"
mkdir -p "$data_dir/shaders" "$data_dir/playlists"

# Install shader packs
if [[ -d "$repo_path/shaders" ]]; then
  for entry in "$repo_path"/shaders/*; do
    [[ -e "$entry" ]] || continue
    name=$(basename "$entry")
    
    # Skip legacy playlists directory if present in shaders/
    if [[ "$name" == "playlists" ]]; then
      continue
    fi
    
    if [[ -d "$entry" ]]; then
      dest="$data_dir/shaders/$name"
      rm -rf "$dest"
      mkdir -p "$dest"
      tar -C "$entry" -cf - . | tar -C "$dest" -xf -
      echo "[wallshader-installer]   shader pack: $name"
    fi
  done
else
  echo "[wallshader-installer] Warning: no shaders directory found in source" >&2
fi

# Install playlists
if [[ -d "$repo_path/playlists" ]]; then
  for playlist in "$repo_path"/playlists/*.toml; do
    [[ -e "$playlist" ]] || continue
    name=$(basename "$playlist")
    dest="$data_dir/playlists/$name"
    install -m 0644 "$playlist" "$dest"
    echo "[wallshader-installer]   playlist: $name"
  done
else
  echo "[wallshader-installer] Warning: no playlists directory found in source" >&2
fi

bin_path=$(command -v wallshader 2>/dev/null || true)
if [[ -z "$bin_path" && -n "$prefix" ]]; then
  bin_path="$prefix/bin/wallshader (add to PATH)"
fi

echo "[wallshader-installer] wallshader installation complete"
if [[ -n "$bin_path" ]]; then
  echo "[wallshader-installer] Binary location: $bin_path"
else
  echo "[wallshader-installer] Binary installed; ensure your cargo bin directory is on PATH."
fi
echo "[wallshader-installer] Shader assets installed to: $data_dir"

echo "[wallshader-installer] Run 'wallshader defaults where' to verify configuration."
