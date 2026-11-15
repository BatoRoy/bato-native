#!/usr/bin/env bash
# Regenerate patched/ from upstream sources + patches/, so the tree is
# reproducible:
#
#   upstream (crates.io .crate, or iced's winit fork at a pinned commit)
#   + patches/<crate>*.patch (in the order listed below)
#   + patches/tripwires/<crate>.rs appended to src/lib.rs (the BATO_PATCH
#     constant bato-kit references, so an unpatched build fails to compile)
#
#   scripts/regen-patched.sh            # rewrite patched/ in place
#   scripts/regen-patched.sh /tmp/out   # write elsewhere (dry run), then:
#   diff -r /tmp/out patched            # must print nothing
#
# .crate files come from ~/.cargo/registry/cache when present (offline),
# else from static.crates.io; both are checked against the sha256 below.
# The winit fork is cloned from $WINIT_REPO (default: GitHub).
#
# To change a patch: edit the crate under patched/, `diff -ru` it against a
# fresh upstream copy (plus the patches listed before it) into
# patches/<crate>.patch (without the tripwire), rerun this script and check
# `git diff patched/` is empty. A crate with several patches (winit) gets
# each one on top of the previous.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$here/patched}
mkdir -p "$out"
out=$(cd "$out" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

WINIT_REPO=${WINIT_REPO:-https://github.com/iced-rs/winit.git}
WINIT_REV=48116469f15d3bc52b5e56bb516694be8882870d

fetch_crate() { # name version sha256 -> path of the .crate
  local name=$1 ver=$2 sum=$3 file
  file=$(ls "$HOME"/.cargo/registry/cache/*/"$name-$ver.crate" 2>/dev/null | head -n1 || true)
  if [ -z "$file" ]; then
    file="$work/$name-$ver.crate"
    curl -sSfL "https://static.crates.io/crates/$name/$name-$ver.crate" -o "$file"
  fi
  echo "$sum  $file" | sha256sum -c --quiet - >&2
  echo "$file"
}

finish() { # dir dest-name patch...
  local dir=$1 dest=$2 patch
  shift 2
  for patch in "$@"; do
    git -C "$dir" apply --whitespace=nowarn "$here/patches/$patch"
  done
  cat "$here/patches/tripwires/$dest.rs" >> "$dir/src/lib.rs"
  rm -rf "${out:?}/$dest"
  mv "$dir" "$out/$dest"
  echo "regenerated $out/$dest"
}

crate() { # name version sha256 patch...
  local name=$1 ver=$2 sum=$3 file
  shift 3
  file=$(fetch_crate "$name" "$ver" "$sum")
  tar -xzf "$file" -C "$work"
  local dir="$work/$name-$ver"
  # git apply needs a repository to resolve paths relative to the crate root.
  git -C "$dir" init -q
  finish "$dir" "$name" "$@"
  rm -rf "$out/$name/.git"
  # The crate's own .gitignore drops Cargo.lock from the bato-native tree.
  if grep -qx '/\?Cargo.lock' "$out/$name/.gitignore" 2>/dev/null; then
    rm -f "$out/$name/Cargo.lock"
  fi
}

crate cosmic-text 0.15.0 173852283a9a57a3cbe365d86e74dc428a09c50421477d5ad6fe9d9509e37737 \
  cosmic-text-0.15.0-freetype-light-hinting.patch cosmic-text-0.15.0-family-fallback.patch
crate cryoglyph 0.1.0 08bc795bdbccdbd461736fb163930a009da6597b226d6f6fce33e7a8eb6ec519 \
  cryoglyph-0.1.0-skia-gamma.patch
crate iced_wgpu 0.14.0 ff144a999b0ca0f8a10257934500060240825c42e950ec0ebee9c8ae30561c13 \
  iced_wgpu-0.14.0-surface-copy-src.patch
crate iced_winit 0.14.1 7589888c8e951899cc688247a69933bb7a0511f0b4b2e122ac3fcd5dacb37f72 \
  iced_winit-0.14.1-wayland-selection.patch

# iced's winit fork (value120 wheel deltas on Wayland), pinned, with smol_str 0.2,
# desktop file drops on Wayland (wl_data_device text/uri-list), keypad keys
# that honour NumLock in key_without_modifiers (X11 and Wayland, xkb), and
# clipboard reads through the file-drop data device (Hyprland sends the
# selection only to a client's first wl_data_device; iced_winit reads it),
# and OS drags of files from a window (bato_drag: a wl_data_source on that
# same data device; on X11 an XDND source; bato-kit's dnd::os_drag starts them).
git clone -q --no-checkout "$WINIT_REPO" "$work/winit"
git -C "$work/winit" -c advice.detachedHead=false checkout -q "$WINIT_REV"
finish "$work/winit" winit iced-winit-fork-smol-str-0.2.patch winit-wayland-file-drop.patch \
  winit-xkb-keypad-numlock.patch winit-wayland-selection.patch winit-wayland-drag-source.patch \
  winit-x11-drag-source.patch
rm -rf "$out/winit/.git"
