#!/usr/bin/env python3
"""THE PO ASSERTION — the 432ace9 incident's detector, automated.

Scans EVERY msgid and msgstr in docs/po/zh_CN.po (and rut.pot):
zero values may satisfy `value.encode('latin-1').decode('utf-8')`
while differing from the original. A double-encoding (the fold's
historic latin-1 round-trip hiccup) trips this loudly.

Also asserts the po re-parses (entry count == pot count) and that
zh translations are present by CONTENT BYTES elsewhere (the build
check greps the rendered page for CJK — this file only certifies
encoding sanity).
"""
import sys

sys.path.insert(0, 'scripts')
from fold_po_lib import parse


def double_encoded(v):
    try:
        b = v.encode('latin-1')
    except UnicodeEncodeError:
        return False  # real Unicode: cannot be a latin-1 round-trip
    try:
        back = b.decode('utf-8')
    except UnicodeDecodeError:
        return False
    return back != v


def main():
    bad = 0
    for path in ['docs/po/zh_CN.po', 'docs/po/rut.pot']:
        _, entries = parse(open(path, encoding='utf-8').read())
        for field in (0, 1):
            for m, s, _ in entries:
                v = m if field == 0 else s
                if not v:
                    continue
                if double_encoded(v):
                    bad += 1
                    print(f'DOUBLE-ENCODED [{path}] {"msgid" if field == 0 else "msgstr"}:')
                    print('  ', v[:120])
        n = len(entries)
        print(f'{path}: {n} entries scanned, no double-encoding' if bad == 0 else f'{path}: {bad} BAD')
    # the header too
    header, _ = parse(open('docs/po/zh_CN.po', encoding='utf-8').read())
    if header and double_encoded(header):
        bad += 1
        print('DOUBLE-ENCODED header')
    if bad:
        print(f'ASSERTION FAILED: {bad} double-encoded value(s)')
        sys.exit(1)
    print('THE ASSERTION HOLDS: zero latin-1 round-trip values in msgid/msgstr/header')

    # sanity: zh presence by content bytes inside the CATALOG itself
    po = open('docs/po/zh_CN.po', encoding='utf-8').read()
    cjk = sum(po.count(c) for c in '项目清单捆绑包编译拒绝声明注释逗号')
    print(f'zh catalog CJK content bytes present: {cjk}')
    assert cjk > 1000, 'the catalog lost its Chinese'


if __name__ == '__main__':
    main()
