set shell := ["bash", "-cu"]

# Install rustfmt/clippy components and pre-fetch dependencies
setup:
	rustup component add rustfmt clippy
	cargo fetch

# Format check and lint the entire workspace
check:
	cargo fmt --all --check
	cargo clippy --all-targets --all-features -- -D warnings

# Run the default windowed demo shader
run-demo:
	cargo run -p wax11 -- --window --shadertoy https://www.shadertoy.com/view/3dXyWj

# Run the playlist sample configuration
run-playlist:
	cargo run -p wax11 -- --playlist workspaces.toml

# Mirror the GitHub Actions workflow (build + tests + clippy + format check)
validate:
	cargo build --verbose
	cargo test --verbose
	cargo clippy --all-targets --all-features -- -D warnings
	cargo fmt --all --check

# Create a new release tag and push it (use: just release 0.9.2)
release VERSION:
	#!/usr/bin/env bash
	set -euo pipefail
	
	# Validate version format
	if [[ ! "{{VERSION}}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
		echo "Error: Version must be in format X.Y.Z (e.g., 0.9.2)"
		exit 1
	fi
	
	TAG="v{{VERSION}}"
	
	# Always delete release first if it exists (using gh CLI or warn user)
	if command -v gh >/dev/null 2>&1; then
		echo "Checking for existing GitHub release $TAG..."
		if gh release view "$TAG" >/dev/null 2>&1; then
			echo "Deleting existing GitHub release $TAG..."
			gh release delete "$TAG" -y
		fi
	else
		echo "⚠️  Warning: gh CLI not found."
		echo "Please manually delete any existing release at:"
		echo "https://github.com/NickCody/wax11/releases/tag/$TAG"
		read -p "Press Enter when ready to continue..."
	fi
	
	# Check if tag already exists locally
	if git rev-parse "$TAG" >/dev/null 2>&1; then
		echo "Deleting local tag $TAG..."
		git tag -d "$TAG"
	fi
	
	# Check if tag exists remotely
	if git ls-remote --tags origin | grep -q "refs/tags/$TAG"; then
		echo "Deleting remote tag $TAG..."
		git push origin ":refs/tags/$TAG"
		# Wait a moment for GitHub to process the deletion
		sleep 2
	fi
	
	echo "Creating and pushing tag $TAG..."
	git tag "$TAG"
	git push origin "$TAG"
	
	echo "✅ Release $TAG created and pushed!"
	echo "🚀 Check the build progress at: https://github.com/NickCody/wax11/actions"
