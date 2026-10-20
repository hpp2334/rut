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
    # the std-v11 publish (the v11-wire + v22-templates repack): the
    # pin rows ride the new tag, every envelope sha moves with the new
    # bytes (the whole 16-artifact set repacked)
    (re.compile(r'\bstd-v10\b'), 'std-v11'),
    (re.compile(r'\b1bbd861905b6cc09435833d18122f0740be14b7b4bc306821634fb88d38e6f83\b'),
     '249aba803d8a70f05952041f667b5897f4733257f00d5d7b73c6032f4ba70373'),
    (re.compile(r'\b3617601203bdd8d71eb34663ee7cc9790065b2ad49ff2e7ef5a77369b44ed039\b'),
     '811e6f5b98ebb2816ea21782e42441183a1c5d262850567164540cc3d7801467'),
    (re.compile(r'\b62b92c34beeda942ffb5cbf8ca84146c56bb33131ac3c0c0a0f8f9ebd643aa02\b'),
     '8b9b62ef68994115db7a8e75bf410fd4ab3a6c0213733b71df1001b4db83a7b1'),
    (re.compile(r'\bc165618cb6b1cd44efe00dacfeea3e53d9420f7037936c7018d66cb777f9dc5a\b'),
     '3494e2554e369119fa460213c00b3384ab6318892d09cc236cdea359c5388677'),
    (re.compile(r'\b4197b149b08f1963aa42374aa988be162411dd244f07193d885f337c8a761ac6\b'),
     '7e9e7f58582ae0a3cd5b4ae76a16b7051c0c79f9479edc34a3ab8261ee250205'),
    (re.compile(r'\b2a45f7279797446bc7bb8e1ed31f9ac0de275e1d615f50f5439ab87a5e1c4f8a\b'),
     '48493535461a15bbe7bcdea881e221cce62354ee92651c527533241853a3c730'),
    (re.compile(r'\bb22614a88d62f711ec061cebd573b452f93c6ef85b96fbed8531cf012aae8353\b'),
     'a3b44897107b3a04aa605ad94f674dedd254755f0310448feb8d1c5102cfa89b'),
    (re.compile(r'\b61974c5c60e8c85fe9a155b46cf965dfe2ba0dfcd0451d0d46d55145cabd5407\b'),
     '0ded688375f71dc4241096faf4ac07bd5154f765705145908589d911fef918d0'),
    (re.compile(r'\bc27400b68f9bb6ce4634f3b59fdcbdb76b974c0d86abfe57cf1aadc8b6c9bb6e\b'),
     '1c152f28ff2d55b9dbd9868c2abe15c7116840d6a4cc8bcd4f2cdf520f848dfd'),
    # the std-v12 publish (the declaration-entry repack): the pin rows
    # ride the new tag, every envelope sha moves with the new bytes
    # (the whole 16-artifact set repacked)
    (re.compile(r'\bstd-v11\b'), 'std-v12'),
    (re.compile(r'\b0ded688375f71dc4241096faf4ac07bd5154f765705145908589d911fef918d0\b'),
     '2d208210ab703ab5f2d9e6a0f8f4fb9e3aa5ea3b615d79db40ef58de306cb5f1'),
    (re.compile(r'\b48493535461a15bbe7bcdea881e221cce62354ee92651c527533241853a3c730\b'),
     '87f68787d1d1c41a33dd78ee6702b295d4b7a8e2aecd25b4b3a14a630ec83699'),
    (re.compile(r'\ba3b44897107b3a04aa605ad94f674dedd254755f0310448feb8d1c5102cfa89b\b'),
     '5d949a0714d199e3b7bee2f4b380a66dc84e9326e0144772f9bec357616e2ddd'),
    (re.compile(r'\b3494e2554e369119fa460213c00b3384ab6318892d09cc236cdea359c5388677\b'),
     'c85f3449e5097a0119302f1da1c43487ce0c36ea81515e00515a0c04f21d50bd'),
    (re.compile(r'\b7e9e7f58582ae0a3cd5b4ae76a16b7051c0c79f9479edc34a3ab8261ee250205\b'),
     '832c202636c3b8108d7540018374618c7cf79131ee4c1f595bd741314b2c8137'),
    (re.compile(r'\b811e6f5b98ebb2816ea21782e42441183a1c5d262850567164540cc3d7801467\b'),
     'ede048a81b8319f0f064bc87dcd2c9c5ded74abb928a91c0dd10e6de0f80ea82'),
    (re.compile(r'\b8b9b62ef68994115db7a8e75bf410fd4ab3a6c0213733b71df1001b4db83a7b1\b'),
     '10b8cbbca24f301495eef7dddda208a997f1698d1172bd5cc41891cd0ead3b21'),
    (re.compile(r'\b249aba803d8a70f05952041f667b5897f4733257f00d5d7b73c6032f4ba70373\b'),
     '63ddd765287243ef45f23105471ff3eae5c5a0ac1d521a3d22636745fd5e72d8'),
    (re.compile(r'\b1c152f28ff2d55b9dbd9868c2abe15c7116840d6a4cc8bcd4f2cdf520f848dfd\b'),
     'e47c5b3d4805e0d38407ab260384356dc8ffc76bc10e9554095f50a5d2184df4'),
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
