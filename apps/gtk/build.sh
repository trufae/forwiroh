#!/bin/sh
set -eu
gtk_root=$(CDPATH= cd "$(dirname "$0")" && pwd)
repo_root=$(CDPATH= cd "$gtk_root/../.." && pwd)
gtk_build_dir="${FORWIROH_GTK_BUILD_DIR:-$gtk_root/build}"
export PATH="$HOME/.cargo/bin:$PATH"
if [ "$(uname -s)" = Darwin ]; then
  export MACOSX_DEPLOYMENT_TARGET=14.0
  export CFLAGS="${CFLAGS:-} -mmacosx-version-min=14.0"
fi
cd "$repo_root"
cargo build --locked --release -p forwiroh
if [ ! -f "$gtk_build_dir/build.ninja" ]; then
  meson setup "$gtk_build_dir" "$gtk_root" "$@"
fi
meson compile -C "$gtk_build_dir"
echo "Built $gtk_build_dir/forwiroh-gtk"
