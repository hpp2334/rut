#!/usr/bin/env bash
# Build the vendored QuickJS-ng `qjs` binary into benches/.tools/qjs.
#
# QuickJS is vendored as a git submodule at benches/tools/quickjs-ng
# (pinned tag; see .gitmodules). If the submodule is not checked out,
# this falls back to a shallow clone of the same pinned tag under
# benches/.tools/src. Nothing is installed system-wide.
#
# quickjs-ng's CMake build is not used: all generated sources are checked
# in, so it compiles directly with a C compiler (no cmake dependency).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCHES="$(cd "$HERE/.." && pwd)"
TOOLS="$BENCHES/.tools"
QJS="$TOOLS/qjs"
SRC="$HERE/quickjs-ng"
PINNED_TAG="v0.16.2"
CC="${CC:-gcc}"

if [[ -x "$QJS" ]]; then
  echo "qjs: already built at $QJS"
  exit 0
fi

mkdir -p "$TOOLS"

if [[ ! -f "$SRC/qjs.c" ]]; then
  echo "qjs: submodule missing — shallow-cloning $PINNED_TAG"
  rm -rf "$TOOLS/src/quickjs-ng"
  mkdir -p "$TOOLS/src"
  git clone --depth 1 --branch "$PINNED_TAG" \
    https://github.com/quickjs-ng/quickjs.git "$TOOLS/src/quickjs-ng"
  SRC="$TOOLS/src/quickjs-ng"
fi

echo "qjs: building $PINNED_TAG from $SRC with $CC"
"$CC" -O2 -D_GNU_SOURCE -DQJS_BUILD_LIBC -I"$SRC" -o "$QJS" \
  "$SRC/qjs.c" \
  "$SRC/quickjs.c" \
  "$SRC/quickjs-libc.c" \
  "$SRC/libregexp.c" \
  "$SRC/libunicode.c" \
  "$SRC/dtoa.c" \
  "$SRC/gen/repl.c" \
  "$SRC/gen/standalone.c" \
  -lm -lpthread -ldl

echo "qjs: built $QJS"
