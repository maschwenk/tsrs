"""A small TypeScript tokenizer for the mutation harness.

It is not a full scanner: it only needs to be exact about where comments, strings,
template literals and regular expressions start and end, so that mutators never
edit inside them by accident, and it pairs brackets so that mutators can reason
about nesting. `>` is always a single token (generic closers must not merge into `>>`).
"""

from dataclasses import dataclass

KEYWORDS = {
    "abstract", "as", "async", "await", "break", "case", "catch", "class", "const", "continue",
    "debugger", "declare", "default", "delete", "do", "else", "enum", "export", "extends", "false",
    "finally", "for", "from", "function", "get", "if", "implements", "import", "in", "instanceof",
    "interface", "is", "keyof", "let", "new", "null", "of", "private", "protected", "public",
    "readonly", "return", "satisfies", "set", "static", "super", "switch", "this", "throw", "true",
    "try", "type", "typeof", "undefined", "var", "void", "while", "with", "yield", "infer", "unique",
    "override", "accessor", "using", "declare", "module", "namespace", "asserts", "never", "unknown",
    "any", "string", "number", "boolean", "symbol", "object", "bigint",
}

PUNCTS = [
    "...", "===", "!==", "**=", "<<=", "&&=", "||=", "??=",
    "=>", "==", "!=", "<=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=",
    "&=", "|=", "^=", "<<", "**",
]

REGEX_PREV_PUNCT = set("( , = : [ ! & | ? { } ; + - * % < > ~ ^ => == === != !== && || ?? += -= *= %= &= |= ^= <= ... **".split())
REGEX_PREV_KW = {"return", "typeof", "case", "do", "else", "in", "of", "new", "delete", "void", "throw",
                 "instanceof", "yield", "await"}


@dataclass
class Tok:
    kind: str  # id, num, str, tmpl_full, tmpl_head, tmpl_mid, tmpl_tail, punct, regex
    text: str
    start: int
    end: int
    line: int  # 0-based line of start
    nl_before: bool  # a line break between the previous token and this one


class LexError(Exception):
    pass


def tokenize(src: str):
    toks = []
    comments = []
    i = 0
    n = len(src)
    line = 0
    nl = True
    brace_stack = []  # 'b' for '{', 't' for template substitution

    def prev_sig():
        return toks[-1] if toks else None

    def scan_template_part(j):
        # j points just after '`' or '}' ; returns (end_index, ended_with_backtick)
        while j < n:
            c = src[j]
            if c == "\\":
                j += 2
                continue
            if c == "`":
                return j + 1, True
            if c == "$" and j + 1 < n and src[j + 1] == "{":
                return j + 2, False
            j += 1
        raise LexError("unterminated template")

    while i < n:
        c = src[i]
        if c == "\n":
            line += 1
            nl = True
            i += 1
            continue
        if c in " \t\r\f\v﻿   ":
            i += 1
            continue
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            j = src.find("\n", i)
            if j < 0:
                j = n
            comments.append((i, j))
            i = j
            continue
        if c == "/" and i + 1 < n and src[i + 1] == "*":
            j = src.find("*/", i + 2)
            if j < 0:
                raise LexError("unterminated comment")
            j += 2
            line += src.count("\n", i, j)
            if "\n" in src[i:j]:
                nl = True
            comments.append((i, j))
            i = j
            continue
        start = i
        startline = line
        if c.isalpha() or c in "_$" or c == "#" or ord(c) > 127:
            j = i + 1
            while j < n and (src[j].isalnum() or src[j] in "_$" or (ord(src[j]) > 127 and src[j].isalpha())):
                j += 1
            toks.append(Tok("id", src[i:j], i, j, startline, nl))
            i = j
        elif c.isdigit() or (c == "." and i + 1 < n and src[i + 1].isdigit()):
            j = i + 1
            if c == "0" and j < n and src[j] in "xXoObB":
                j += 1
                while j < n and (src[j].isalnum() or src[j] == "_"):
                    j += 1
            else:
                while j < n and (src[j].isdigit() or src[j] == "_"):
                    j += 1
                if j < n and src[j] == "." and c != ".":
                    j += 1
                    while j < n and (src[j].isdigit() or src[j] == "_"):
                        j += 1
                elif c == ".":
                    pass
                if j < n and src[j] in "eE":
                    k = j + 1
                    if k < n and src[k] in "+-":
                        k += 1
                    if k < n and src[k].isdigit():
                        j = k
                        while j < n and src[j].isdigit():
                            j += 1
                if j < n and src[j] == "n":
                    j += 1
            toks.append(Tok("num", src[i:j], i, j, startline, nl))
            i = j
        elif c in "'\"":
            j = i + 1
            while j < n and src[j] != c:
                if src[j] == "\\":
                    j += 1
                elif src[j] == "\n":
                    raise LexError("newline in string")
                j += 1
            if j >= n:
                raise LexError("unterminated string")
            j += 1
            toks.append(Tok("str", src[i:j], i, j, startline, nl))
            i = j
        elif c == "`":
            j, done = scan_template_part(i + 1)
            line += src.count("\n", i, j)
            toks.append(Tok("tmpl_full" if done else "tmpl_head", src[i:j], i, j, startline, nl))
            if not done:
                brace_stack.append("t")
            i = j
        elif c == "}" and brace_stack and brace_stack[-1] == "t":
            brace_stack.pop()
            j, done = scan_template_part(i + 1)
            line += src.count("\n", i, j)
            toks.append(Tok("tmpl_tail" if done else "tmpl_mid", src[i:j], i, j, startline, nl))
            if not done:
                brace_stack.append("t")
            i = j
        elif c == "/" and _regex_allowed(prev_sig()):
            j = i + 1
            in_class = False
            while j < n:
                d = src[j]
                if d == "\\":
                    j += 2
                    continue
                if d == "\n":
                    raise LexError("newline in regex")
                if in_class:
                    if d == "]":
                        in_class = False
                elif d == "[":
                    in_class = True
                elif d == "/":
                    break
                j += 1
            j += 1
            while j < n and (src[j].isalnum() or src[j] == "_"):
                j += 1
            toks.append(Tok("regex", src[i:j], i, j, startline, nl))
            i = j
        else:
            m = None
            if c == "?" and src.startswith("?.", i) and i + 2 < n and src[i + 2].isdigit():
                m = "?"
            else:
                for p in PUNCTS:
                    if src.startswith(p, i):
                        m = p
                        break
            if m is None:
                m = c
            if m == "{":
                brace_stack.append("b")
            elif m == "}":
                if brace_stack:
                    brace_stack.pop()
            toks.append(Tok("punct", m, i, i + len(m), startline, nl))
            i += len(m)
        nl = False
    return toks, comments


def _regex_allowed(prev):
    if prev is None:
        return True
    if prev.kind == "punct":
        return prev.text in REGEX_PREV_PUNCT and prev.text not in (")", "]", "}")
    if prev.kind == "id":
        return prev.text in REGEX_PREV_KW
    return False


OPEN = {"(": ")", "[": "]", "{": "}"}


def pair_brackets(toks):
    """Returns (match, parent): match[i] = index of the partner bracket (or -1);
    parent[i] = index of the innermost enclosing opener (or -1). Template heads/mids open,
    mids/tails close."""
    n = len(toks)
    match = [-1] * n
    parent = [-1] * n
    stack = []
    for i, t in enumerate(toks):
        parent[i] = stack[-1] if stack else -1
        k = t.kind
        tx = t.text
        if k == "punct" and tx in OPEN or k == "tmpl_head":
            stack.append(i)
        elif k == "punct" and tx in (")", "]", "}") or k == "tmpl_tail":
            if not stack:
                raise LexError("unbalanced close at %d" % t.start)
            o = stack.pop()
            ot = toks[o]
            if k == "tmpl_tail":
                ok = ot.kind in ("tmpl_head", "tmpl_mid")
            else:
                ok = ot.kind == "punct" and OPEN[ot.text] == tx
            if not ok:
                raise LexError("mismatched bracket at %d" % t.start)
            match[o] = i
            match[i] = o
            parent[i] = stack[-1] if stack else -1
        elif k == "tmpl_mid":
            if not stack:
                raise LexError("unbalanced template")
            o = stack.pop()
            match[o] = i
            match[i] = o
            parent[i] = stack[-1] if stack else -1
            stack.append(i)
    if stack:
        raise LexError("unclosed bracket")
    return match, parent
