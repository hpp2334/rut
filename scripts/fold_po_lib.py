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
# cutover's textual shapes, applied identically to msgid and msgstr;
# the vocabulary purge's shape: the legacy word IS `struct` now;
# the push-contract rename: the trait's name IS `Iterable` now;
# the main-convention retirement: the code-span shape IS `entry fn`
# now — `pub fn main` was the retired spelling of the same decl)
MECH = [
    (re.compile(r'\bdataclasses\b'), 'structs'),
    (re.compile(r'\bdataclass\b'), 'struct'),
    (re.compile(r'数据类'), '结构体'),
    (re.compile(r'rut\.json(?!c)'), 'rut.jsonc'),
    (re.compile(r'\bv7\b'), 'v9'),
    (re.compile(r'\bv8\b'), 'v10'),
    (re.compile(r'format_version 7\b'), 'format_version 9'),
    (re.compile(r'format_version 8\b'), 'format_version 10'),
    (re.compile(r'\bIterator\b'), 'Iterable'),
    # the std-v5 publish: the book's pin rows ride the new tag spelling
    (re.compile(r'\bstd-v3\b'), 'std-v5'),
    # the std-v10 publish (the mut-law repack): the pin rows ride the
    # new tag, and the pouch/json rows' shas move with the new bytes
    (re.compile(r'\bstd-v9\b'), 'std-v10'),
    (re.compile(r'\b2f6e6a201083c012edb5362d1ed79bce409747d68c48ff48f23383be4cc54466\b'),
     '1bbd861905b6cc09435833d18122f0740be14b7b4bc306821634fb88d38e6f83'),
    (re.compile(r'\ba39d4bfbd89b1046e3505147811badc07fb4c7abcbde4d4f946419d163245983\b'),
     '3617601203bdd8d71eb34663ee7cc9790065b2ad49ff2e7ef5a77369b44ed039'),
    # the main convention retires: `entry fn main` is the same decl's
    # spelling now (drop `pub`, add `entry`)
    (re.compile(r'\bpub fn main\b'), 'entry fn main'),
]


def mech(s):
    for rx, rep in MECH:
        s = rx.sub(rep, s)
    return s


def mech_diff(old_id, new_id):
    """True iff new_id == mech(old_id) and they differ."""
    return old_id != new_id and mech(old_id) == new_id
