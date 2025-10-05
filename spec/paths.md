# WallShader Path & Handle Specification

## Remaining Tasks

### Advanced Handle Schemes
- **`shader://` scheme**: Currently uses bare names instead of explicit `shader://pack` syntax
- **`playlist://` scheme**: Playlists are referenced by filename/path, not via `playlist://name` scheme
- **Flattened playlist layout**: Playlists remain in `playlists/` subdirectory rather than top-level `*.toml` files

### Enhanced Path Features
- **Search path configuration**: More granular control over search order and custom paths
- **Builtin shader schemes**: Reserved `builtin://` for hard-coded demo shaders
- **Advanced scheme extensibility**: Generic scheme parsing for future integrations (`steam://`, `gallery://`)

### Developer Experience
- **Better error messages**: Include search roots checked when resolution fails
- **Path diagnostics**: Enhanced debugging tools for path resolution issues