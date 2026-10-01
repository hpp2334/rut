"""Shared po plumbing: the parser, quoting, and the mechanical cutover rules."""

import re


def parse(text):
    """PO parser → (header_msgstr, [(msgid, msgstr, refs)]).

    `refs` is the verbatim `#:` occurrence block (with trailing newline,
    '' when absent). The header is the (msgid "", msgstr "") entry's
    value; it is not in the list.
    """
    entries = []
    header = None
    lines = text.split('\n')
    i = 0
    while i < len(lines):
        line = lines[i]
        if line.startswith('msgid '):
            refs = []
            j = i
            while j > 0 and lines[j - 1].startswith('#: '):
                j -= 1
                refs.insert(0, lines[j] + '\n')
            # msgid block
            buf = [line[len('msgid '):]] if line != 'msgid ""' else []
            i += 1
            while i < len(lines) and lines[i].startswith('"'):
                buf.append(lines[i])
                i += 1
            msgid = unquote(buf)
            # msgstr block
            if i < len(lines) and lines[i].startswith('msgstr'):
                sbuf = [lines[i][len('msgstr'):].strip()] if lines[i][len('msgstr'):].strip() else []
                i += 1
                while i < len(lines) and lines[i].startswith('"'):
                    sbuf.append(lines[i])
                    i += 1
                msgstr = unquote(sbuf)
            else:
                msgstr = ''
            if msgid == '' and header is None:
                header = msgstr
            else:
                entries.append((msgid, msgstr, ''.join(refs)))
            continue
        i += 1
    return header, entries


def unquote(buf):
    s = ''.join(x.strip() for x in buf)
    out = []
    i = 0
    while i < len(s):
        c = s[i]
        if c == '"':
            i += 1
            continue
        if c == '\\' and i + 1 < len(s):
            n = s[i + 1]
            out.append({'n': '\n', 't': '\t', '"': '"', '\\': '\\'}.get(n, '\\' + n))
            i += 2
            continue
        out.append(c)
        i += 1
    return ''.join(out)


def quote(s):
    """PO quote: single line when short and newline-free, else wrapped."""
    if '\n' not in s and len(s) < 76:
        return [f'"{esc(s)}"']
    parts = s.split('\n')
    out = ['""']
    for j, p in enumerate(parts):
        if j < len(parts) - 1:
            out.append(f'"{esc(p)}\\n"' if p else '"\\n"')
        elif p:
            out.append(f'"{esc(p)}"')
    return out


def esc(s):
    return (s.replace('\\', '\\\\')
             .replace('"', '\\"')
             .replace('\t', '\\t')
             .replace('\n', '\\n'))


# the mechanical cutover — the ONLY auto-carried diffs (the JSONC
# cutover's textual shapes, applied identically to msgid and msgstr)
MECH = [
    (re.compile(r'rut\.json(?!c)'), 'rut.jsonc'),
    (re.compile(r'\bv7\b'), 'v9'),
    (re.compile(r'\bv8\b'), 'v10'),
    (re.compile(r'format_version 7\b'), 'format_version 9'),
    (re.compile(r'format_version 8\b'), 'format_version 10'),
]


def mech(s):
    for rx, rep in MECH:
        s = rx.sub(rep, s)
    return s


def mech_diff(old_id, new_id):
    """True iff new_id == mech(old_id) and they differ."""
    return old_id != new_id and mech(old_id) == new_id
