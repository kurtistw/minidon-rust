#!/bin/sh
# Make the Python oracle: a git archive copy of the reference repo (read only, never written),
# with dependencies from `uv sync --frozen` inside the COPY. Usage: setup.sh <reference repo> <sha> <dest>
# Prints nothing on success. The reference repo is only read by `git archive`.
set -eu
repo="$1"; sha="$2"; dest="$3"
if [ -f "$dest/.oracle-ready" ]; then exit 0; fi
rm -rf "$dest"
mkdir -p "$dest"
git -C "$repo" archive "$sha" | tar -x -C "$dest"
cd "$dest"
PYTHONDONTWRITEBYTECODE=1 uv sync --frozen --quiet
printf '%s\n' "$sha" > .oracle-ready
