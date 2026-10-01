#!/usr/bin/env python3
"""Report the po delta: which msgids are new, mechanically-renamed, or changed."""
import re, sys
sys.path.insert(0, 'scripts')
from fold_po_lib import parse, mech, mech_diff

POT = 'docs/po/rut.pot'
PO = 'docs/po/zh_CN.po'

pot_header, pot_entries = parse(open(POT, encoding='utf-8').read())
pot_entries = [(m, s) for m, s, _ in pot_entries]
po_header, po_entries = parse(open(PO, encoding='utf-8').read())
po_entries = [(m, s) for m, s, _ in po_entries]
old = dict(po_entries)
old_ids = set(old)

new = []
mech_carried = []
changed = []
gone = []
for msgid, _ in pot_entries:
    if msgid in old:
        continue
    hit = [o for o in old_ids if mech_diff(o, msgid)]
    if hit:
        mech_carried.append((hit[0], msgid))
    else:
        new.append(msgid)
for o in sorted(old_ids):
    if o not in {m for m, _ in pot_entries} and not any(mech_diff(o, m) for m, _ in pot_entries):
        gone.append(o)

# "changed" = new msgids that look like edits of a gone id (share a 40-char prefix)
print(f'pot entries: {len(pot_entries)} | old: {len(old)}')
print(f'mech-carried: {len(mech_carried)}')
print(f'brand-new: {len(new)}')
print(f'gone (no mech heir): {len(gone)}')
print()
print('=== mech-carried (old → new), first 10 ===')
for o, n in mech_carried[:10]:
    print(' OLD:', o[:100])
    print(' NEW:', n[:100])
print()
print('=== gone ids (had translation?) — first 30 ===')
t = 0
for o in gone:
    had = bool(old[o])
    if had: t += 1
print(f'gone-with-translation: {t}/{len(gone)}')
for o in gone[:30]:
    print(('  [T] ' if old[o] else '  [ ] ') + o[:110])
print()
print('=== brand-new — first 60 ===')
for n in new[:60]:
    print('  +', n[:110])
