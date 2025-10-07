# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Branch protection documentation in `.github/BRANCH_PROTECTION.md` explaining how to configure GitHub settings to require CI checks before merging PRs
- `CONTRIBUTING.md` with comprehensive contribution guidelines, PR process, and coding standards
- Pull request template in `.github/pull_request_template.md` to standardize PR submissions
- CI status badge in README to show build status at a glance

## [0.9.1] - 2025-09-28
### Added
- MIT license file and shared workspace metadata for repository, keywords, and homepage.
- `rust-toolchain.toml` pinning the stable toolchain with `rustfmt`/`clippy` components.
- `just` helper recipes for setup, checks, and demo runs.
- README guidance for installing with `cargo install --git` and quick-start cloning instructions.
