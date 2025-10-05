# Shader Compilation Spec

## Remaining Tasks

### Documentation Enhancements
- Produce a dedicated user guide (`docs/`) that walks through manifest fields, playlist overrides, and troubleshooting examples
- Keep README as the quick reference once the dedicated guide exists

### Advanced Color Pipeline Features
- **Per-item overrides**: Consider extending multi-playlist TOML (and eventually manifests) with per-pass or per-playlist color overrides, honoring the CLI > playlist > manifest > default hierarchy
- **Shader wrapper helpers**: Add optional macros or uniforms so advanced shaders can explicitly request gamma↔linear conversions without retooling the pipeline

### Quality Assurance Tools
- **Validation tooling**: Capture side-by-side screenshots or numeric comparisons against Shadertoy for both gamma and linear modes to document expected output