# Lambdash Path & Handle Specification

## Goals
- Make asset resolution predictable by treating anything that looks like a filesystem path as literal (shell semantics apply).
- Reserve shorthand for explicit URI-style handles so we can expand behaviour without ambiguity.
- Flatten the default data layout so shader packs live directly under the data directory root and playlists are top-level `*.toml` files.
- Provide a consistent workflow for developers (working from the repo tree) and end users (working from their data directory).

## Terminology
- **PWD** — process working directory when `lambdash` starts.
- **AppDirs** — resolved trio of XDG-style roots (`config_dir`, `data_dir`, `cache_dir`).
- **Share Dir** — read-only installation root (e.g. `/usr/share/lambdash`) that ships bundled assets.
- **Repo Assets** — checked-in sample shader packs used during development (`shaders/` in-tree).

## Handle Classes

### 1. Raw Paths (default)
- Detection: any argument that contains a `/` character *after* environment/tilde expansion.
- Expansion: perform `${VAR}`/`$VAR` substitution (error if undefined), then tilde expansion, then interpret per POSIX rules.
- Resolution: no search paths, no inferred subdirectories. The resulting path may be absolute or relative to the PWD.
- Examples: `./simplex-color`, `../share/smoke`, `/opt/lambdash/sunrise`, `~/Shaders/fire/image.glsl`.

### 2. `shadertoy://<ID>`
- Behaviour unchanged: resolve via `ShadertoyClient`, cache under `AppDirs.cache_dir/shadertoy/<ID>`.
- Valid IDs follow ShaderToy rules (alphanumeric + `_`). Reject missing IDs with user-facing errors.

### 3. `local://<name>`
- Represents a shader pack directory containing manifests + GLSL.
- Search order (first match wins):
  1. `AppDirs.data_dir/<name>`
  2. `Share Dir/<name>`
  3. Repo assets when running from a checkout: `REPO_ROOT/shaders/<name>` (only if `REPO_ROOT` detected via `LAMBDASH_DEV_ROOT` or heuristics).
- The resolved path must be a directory; fail if missing or not a dir.

### 4. `playlist://<name>`
- Represents a playlist manifest file named `<name>.toml`.
- Search order (first match wins):
  1. `AppDirs.data_dir/<name>.toml`
  2. `Share Dir/playlists/<name>.toml`
  3. Repo assets: `REPO_ROOT/shaders/playlists/<name>.toml`.
- Reject directories; require readable TOML file.

### 5. Future Schemes
- Reserve `builtin://` for hard-coded demo shaders (optional).
- Keep scheme parsing generic so additional integrations (`steam://`, `gallery://`) can register later.

## Data Directory Layout (post-change)
```
$DATA_DIR/
  simplex-color/            # shader pack directory
    shader.toml
    image.glsl
    textures/
  smoke/
    shader.toml
  playlist-default.toml     # playlist manifest (no playlist/ subdir)
  workspace-rotation.toml
  cache/                    # optional runtime scratch (not copied from repo)
```
- Installer and bootstrap routines copy repo `shaders/<pack>` into `$DATA_DIR/<pack>`.
- Playlist samples move from `shaders/playlists/*.toml` to top-level `$DATA_DIR/*.toml` during the packaging step.
- Runtime should tolerate existing `shaders/` or `playlists/` directories for now but log deprecation.

## Developer Workflow
- Run commands from repo root: `cargo run -p lambdash -- ./shaders/simplex-color` to use in-tree assets literally.
- To test installed layouts, invoke `scripts/install.sh --data-dir ~/.local/share/lambdash` then run `lambdash local://simplex-color`.
- When editing default packs, modify files under `shaders/<pack>` and rerun the installer; the copy will mirror into the flattened data layout.

## User Workflow
- End users rely on `local://` / `playlist://` handles; they edit files under `$DATA_DIR` directly.
- To override a bundled shader, copy the share-dir version into the data dir and continue editing; the search order ensures the user copy wins.
- Bare filesystem paths work for advanced users who keep assets elsewhere (e.g. `lambdash ~/Shaders/custom-pack`).

## Implementation Notes
- Centralize parsing in a `Handle` enum (`RawPath`, `Shadertoy(id)`, `LocalPack(name)`, `Playlist(name)`).
- Enforce "slash means raw" before scheme detection to avoid `local://` being misinterpreted.
- Provide descriptive errors: include the search roots that were checked, and suggest using an explicit scheme when a bare handle is ambiguous.
- Update `scripts/install.sh` to flatten the copy layout (dirs move up one level, playlists emitted as top-level `.toml`).
- Document the final behaviour in `README.md` and `AGENTS.md` once implemented.

## Implementation Phases

### Phase 1 — Resolver & CLI Wiring
- Introduce the `Handle` enum and parser, update CLI argument handling to produce typed handles, and switch consumers (daemon, tests, tools) to use the new abstraction.
- Ensure raw paths execute the literal-path semantics (env expansion + POSIX resolution) and add targeted tests covering edge cases like undefined vars and trailing slashes.
- Gate the existing search logic behind `local://` and `playlist://`, updating error messages to describe the new scheme expectations.

### Phase 2 — Asset Layout & Installer Updates
- Adjust bootstrap/install scripts to copy shader packs directly into `$DATA_DIR/<pack>` and playlists to `$DATA_DIR/*.toml`.
- Update runtime search paths to tolerate legacy trees temporarily (project is unreleased, so migrations can be manual).
- Refresh documentation stubs (`README`, `AGENTS`, sample configs) to advertise the flattened layout and scheme usage.

### Phase 3 — Codebase Sweep & Tooling
- Update internal references, tests, and fixtures to use schemes or literal paths per the spec (e.g., adjust default configs, integration tests, and sample commands).
- Extend developer tooling (e.g., `just`/CI scripts) to exercise both literal-path and scheme-based flows, ensuring regressions are caught.
- Remove any remaining compatibility shims and finalize release notes once the new path model is stable.
