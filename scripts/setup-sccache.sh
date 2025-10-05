#!/bin/bash
# Setup sccache for faster Rust compilation
# Run this once: ./scripts/setup-sccache.sh

set -e

echo "Installing sccache for faster Rust compilation..."

# Check if sccache is already installed
if command -v sccache &> /dev/null; then
    echo "sccache is already installed at: $(which sccache)"
    sccache --version
else
    echo "Installing sccache via cargo..."
    cargo install sccache
fi

# Configure cargo to use sccache
mkdir -p ~/.cargo
cat >> ~/.cargo/config.toml << 'EOF'

# sccache configuration for faster builds
[build]
rustc-wrapper = "sccache"
EOF

echo ""
echo "✅ sccache installed and configured!"
echo ""
echo "To see cache statistics, run: sccache --show-stats"
echo "To clear the cache, run: sccache --zero-stats"
echo ""
echo "Your next cargo build will use sccache for caching."
