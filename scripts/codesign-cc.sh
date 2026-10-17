#!/bin/sh
# Ad-hoc-signing `cc` wrapper (Intel macOS linker-1221.4+).
#
# The new Apple linker dropped `-Wl,-adhoc_sign` (the old spelling this
# repo's cargo configs used), and on Intel rustc signs nothing itself —
# unsigned Mach-O execs consult taskgated, which wedges under concurrent
# unsigned exec load (parallel test runs), stalling every subsequent exec
# by ~26-35s each. So: link with the system cc exactly as rustc asked,
# then ad-hoc sign whatever the `-o` argument named. Signing is best
# effort — a codesign failure must not fail the link.
cc="$CODESIGN_CC"
[ -n "$cc" ] || cc=cc
"$cc" "$@"
rc=$?
[ $rc -eq 0 ] || exit $rc
out=
prev=
for a in "$@"; do
    if [ "$prev" = "-o" ]; then out=$a; fi
    prev=$a
done
if [ -n "$out" ] && [ -f "$out" ]; then
    codesign -f -s - "$out" >/dev/null 2>&1 || true
fi
exit $rc
