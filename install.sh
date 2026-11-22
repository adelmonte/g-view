#!/bin/sh
# Build and install g-view for the current user (PREFIX=/usr/local sudo ./install.sh for system-wide)
set -e
PREFIX="${PREFIX:-$HOME/.local}"
cargo build --release
install -Dm755 target/release/g-view "$PREFIX/bin/g-view"
install -Dm644 dist/g-view.desktop "$PREFIX/share/applications/g-view.desktop"
for d in dist/hicolor/*/apps; do
  size=$(basename "$(dirname "$d")")
  for f in "$d"/*; do
    ext="${f##*.}"; case "$f" in *symbolic*) name=g-view-symbolic;; *) name=g-view;; esac
    install -Dm644 "$f" "$PREFIX/share/icons/hicolor/$size/apps/$name.$ext"
  done
done
gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$PREFIX/share/applications" 2>/dev/null || true
echo "Installed to $PREFIX/bin/g-view"
