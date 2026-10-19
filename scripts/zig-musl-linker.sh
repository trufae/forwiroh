#!/bin/bash
# Zig supplies the musl unwind runtime missing from Ubuntu's musl-gcc linker.
set -euo pipefail
: "${FORWIROH_MUSL_TARGET:?Set by build-swift-core.sh}"
args=()
for arg in "$@"; do
    case "$arg" in
        # Rust's GCC driver flag is not accepted by the Zig driver.
        -Wl,--fix-cortex-a53-843419) ;;
        *) args+=("$arg") ;;
    esac
done
exec zig cc -target "$FORWIROH_MUSL_TARGET" "${args[@]}"
