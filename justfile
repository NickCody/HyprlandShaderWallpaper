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
	cargo run -p wallshader -- --window --shadertoy https://www.shadertoy.com/view/3dXyWj

# Run the playlist sample configuration
run-playlist:
	cargo run -p wallshader -- --playlist workspaces.toml

# Mirror the GitHub Actions workflow (build + tests + clippy)
validate:
	cargo build --verbose
	cargo test --verbose
	cargo clippy --all-targets --all-features -- -D warnings

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
	
	# Check if tag already exists locally
	if git rev-parse "$TAG" >/dev/null 2>&1; then
		echo "Tag $TAG already exists locally. Deleting..."
		git tag -d "$TAG"
	fi
	
	# Check if tag exists remotely
	if git ls-remote --tags origin | grep -q "refs/tags/$TAG"; then
		echo "Tag $TAG exists remotely. Deleting..."
		git push origin ":refs/tags/$TAG"
		
		# Check if release exists and delete it
		if command -v gh >/dev/null 2>&1; then
			echo "Deleting GitHub release $TAG..."
			gh release delete "$TAG" -y 2>/dev/null || echo "No release to delete"
		else
			echo "Warning: gh CLI not found. You may need to manually delete the release at:"
			echo "https://github.com/NickCody/WallShader/releases/tag/$TAG"
			read -p "Press Enter to continue..."
		fi
	fi
	
	echo "Creating and pushing tag $TAG..."
	git tag "$TAG"
	git push origin "$TAG"
	
	echo "✅ Release $TAG created and pushed!"
	echo "🚀 Check the build progress at: https://github.com/NickCody/WallShader/actions"
