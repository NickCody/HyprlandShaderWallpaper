# Contributing to WallShader

Thank you for your interest in contributing to WallShader! This document provides guidelines and instructions for contributing to the project.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Workflow](#development-workflow)
- [Pull Request Process](#pull-request-process)
- [Coding Standards](#coding-standards)
- [Testing](#testing)
- [Commit Messages](#commit-messages)

## Code of Conduct

This project adheres to a code of conduct that all contributors are expected to follow. Be respectful, inclusive, and professional in all interactions.

## Getting Started

### Prerequisites

Before contributing, ensure you have the development environment set up:

1. **Install Rust toolchain**:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
   source "$HOME/.cargo/env"
   rustup component add rustfmt clippy
   ```

2. **Install system dependencies** (Ubuntu/Debian):
   ```bash
   sudo apt update
   sudo apt install -y \
     build-essential pkg-config cmake ninja-build \
     libwayland-dev wayland-protocols libxkbcommon-dev \
     libegl1-mesa-dev libgles2-mesa-dev \
     mesa-vulkan-drivers vulkan-tools libvulkan-dev
   ```

3. **Clone the repository**:
   ```bash
   git clone https://github.com/NickCody/WallShader.git
   cd WallShader
   ```

4. **Install Just (optional but recommended)**:
   ```bash
   cargo install just
   ```

5. **Setup and verify**:
   ```bash
   just setup
   just validate
   ```

## Development Workflow

### Local Development

1. **Create a feature branch**:
   ```bash
   git checkout -b feature/your-feature-name
   ```

2. **Make your changes** following the coding standards below

3. **Run local checks frequently**:
   ```bash
   just check          # Format check + Clippy
   cargo build         # Ensure it compiles
   cargo test          # Run tests
   ```

4. **Test your changes**:
   ```bash
   just run-demo       # Test with windowed demo
   just run-playlist   # Test playlist functionality
   ```

### Before Committing

Always run the full validation suite before committing:

```bash
just validate
```

This runs:
- `cargo build --verbose` - Full workspace build
- `cargo test --verbose` - Complete test suite
- `cargo clippy --all-targets --all-features -- -D warnings` - Linting

All three must pass for CI to succeed.

## Pull Request Process

### Creating a Pull Request

1. **Ensure your branch is up-to-date**:
   ```bash
   git fetch origin
   git rebase origin/main
   ```

2. **Run validation one final time**:
   ```bash
   just validate
   ```

3. **Push your branch**:
   ```bash
   git push origin feature/your-feature-name
   ```

4. **Create the PR**:
   - Go to https://github.com/NickCody/WallShader/pulls
   - Click "New Pull Request"
   - Select your branch
   - Fill in the PR template with:
     - Clear description of changes
     - Motivation/context
     - Testing performed
     - Screenshots/demos if UI-related

### PR Requirements

All Pull Requests must meet the following requirements before merging:

#### ✅ CI Checks Must Pass

The following automated checks must succeed:

1. **Code formatting**: `cargo fmt --all --check`
2. **Linting**: `cargo clippy --all-targets --all-features -- -D warnings`
3. **Build**: `cargo build --verbose`
4. **Tests**: `cargo test --verbose`
5. **AppImage build**: Full release build and packaging

#### ✅ Code Review

- At least one maintainer approval required
- Address all review comments
- Resolve all conversations

#### ✅ Branch Up-to-Date

- Your branch must be rebased on the latest `main`
- No merge conflicts

#### ✅ Documentation

- Update relevant documentation for user-facing changes
- Add/update code comments for complex logic
- Update CHANGELOG.md for notable changes

### If CI Fails

If the CI checks fail on your PR:

1. **Review the failure logs** in the GitHub Actions tab
2. **Fix the issues locally**:
   ```bash
   # For formatting issues
   cargo fmt --all
   
   # For clippy warnings
   cargo clippy --all-targets --all-features -- -D warnings
   
   # For test failures
   cargo test --verbose
   ```
3. **Commit and push the fixes**:
   ```bash
   git add .
   git commit -m "Fix CI issues"
   git push origin feature/your-feature-name
   ```
4. CI will automatically re-run on the new commits

### Addressing Review Comments

1. Make the requested changes in new commits (don't force-push during review)
2. Respond to comments explaining your changes
3. Mark conversations as resolved once addressed
4. Request re-review when ready

## Coding Standards

### Rust Code Style

WallShader follows standard Rust formatting and linting:

- **Formatting**: Use `rustfmt` (enforced by CI)
  ```bash
  cargo fmt --all
  ```

- **Linting**: All Clippy warnings must be resolved (treated as errors in CI)
  ```bash
  cargo clippy --all-targets --all-features -- -D warnings
  ```

- **Comments**: 
  - Add module-level documentation at the top of each file
  - Document public APIs with `///` doc comments
  - Use `//` for inline comments only when necessary
  - Keep comments in sync with code changes

### Code Organization

- **Module structure**: Keep module declarations at the top of files
- **Dependencies**: Only add new dependencies when necessary
- **Error handling**: Use `anyhow::Result` for application errors
- **Logging**: Use the `tracing` crate for structured logging

### Commit Messages

Write clear, descriptive commit messages:

```
Short summary (50 chars or less)

Longer description if needed, explaining:
- What changed
- Why it changed
- Any important context

Fixes #123
```

## Testing

### Writing Tests

- Add tests for all new functionality
- Place unit tests in the same file as the code (in a `#[cfg(test)]` module)
- Place integration tests in `tests/` directories
- Follow existing test patterns in the codebase

### Running Tests

```bash
# Run all tests
cargo test --verbose

# Run specific test
cargo test test_name

# Run tests for a specific crate
cargo test -p wallshader
cargo test -p renderer
cargo test -p shadertoy
```

### Test Coverage

While not strictly required, aim for:
- Critical paths and error handling covered
- Edge cases tested
- Regression tests for bug fixes

## Documentation

### User-Facing Documentation

Update these files when relevant:

- `README.md` - User-facing features, installation, usage
- `CHANGELOG.md` - Notable changes, breaking changes, new features
- `AGENTS.md` - Developer context, architecture notes

### Code Documentation

- Document public APIs with doc comments (`///`)
- Explain complex algorithms or non-obvious logic
- Keep module-level comments current

## Release Process

Releases are managed by maintainers:

1. Version bump in `Cargo.toml`
2. Update `CHANGELOG.md`
3. Create release tag: `just release X.Y.Z`
4. CI automatically builds and publishes the AppImage

Contributors don't need to worry about releases - focus on making great PRs!

## Questions or Issues?

- **Found a bug?** Open an issue: https://github.com/NickCody/WallShader/issues
- **Have a question?** Open a discussion or issue
- **Need help?** Tag maintainers in your PR or issue

## License

By contributing to WallShader, you agree that your contributions will be licensed under the MIT License.
