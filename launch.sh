#!/usr/bin/env bash
# Usage: ./float.sh <program> [args...]
# Example: ./float.sh target/release/hyshadew --shadertoy URL --window

# tweak these to taste
WIDTH=1920
HEIGHT=1080
X=960
Y=540

# build the rule string
RULE="[float;size ${WIDTH} ${HEIGHT};move ${X} ${Y}]"

# pass everything after RULE straight through
hyprctl dispatch -- exec "$RULE $@"

