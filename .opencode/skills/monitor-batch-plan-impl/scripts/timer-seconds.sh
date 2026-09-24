#!/bin/bash
# timer-seconds.sh — the monitor's next sleep: prints "<seconds> <peak|main|none>"
#
# Sleep = the shorter of the routine interval (default 1200 s) and the time
# until the next peak-strategy boundary — 13:50 -> "peak", 18:10 -> "main"
# (workdays Mon–Fri, fixed UTC+8, no DST). Use the output as:
#   none:              sleep 1200 && echo "WAKE_UP_CHECK $(date +%H:%M:%S)"
#   <s> <kind>:        sleep <s> && echo "SWITCH_DUE <kind> $(TZ=Asia/Shanghai date +%H:%M:%S)"
#   0 <kind>:          the boundary just passed (grace window) — send that
#                      switch nudge NOW instead of arming a timer.
set -u
INTERVAL="${1:-1200}"

python3 - "$INTERVAL" <<'PY'
import sys
from datetime import datetime, timedelta, timezone

interval = int(sys.argv[1])
GRACE = 120                                  # absorb timer jitter, seconds
tz8 = timezone(timedelta(hours=8))           # fixed UTC+8
now = datetime.now(tz8)

def boundary(hour, minute):
    cand = now.replace(hour=hour, minute=minute, second=0, microsecond=0)
    due_now = cand < now and (now - cand).total_seconds() <= GRACE
    if not due_now and cand <= now:
        cand += timedelta(days=1)
    while cand.weekday() >= 5:               # Sat/Sun: never due — roll forward
        cand += timedelta(days=1)
        due_now = False
    return cand, due_now

kind, due, due_now = min(
    (("peak", *boundary(13, 50)), ("main", *boundary(18, 10))),
    key=lambda k: k[1])
delta = max(0, int((due - now).total_seconds()))
if due_now:
    print(f"0 {kind}")
else:
    print(f"{min(delta, interval)} {kind if delta < interval else 'none'}")
PY
