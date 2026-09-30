"""Syntactic mutators. Each mutator takes a Ctx and returns candidate edits; an edit
replaces exactly one contiguous span [start, end) of the file with `repl`."""

import random
from dataclasses import dataclass

from tslex import KEYWORDS, pair_brackets, tokenize


@dataclass
class Edit:
    mutator: str
    start: int
    end: int
    repl: str
    note: str = ""


NOT_CALLEES = {"if", "for", "while", "switch", "catch", "function", "return", "typeof", "import",
               "await", "async", "with", "in", "of", "new", "void", "delete", "throw", "yield", "case",
               "else", "do", "keyof", "extends", "implements", "satisfies", "as", "is", "super"}
OBJ_PREV = {"(", ",", "[", "=", "return", "=>", "?", "||", "??", "&&", "...", "yield", "await"}
STMT_BOUND = {";", "{", "}"}


class Ctx:
    def __init__(self, path, src, rng):
        self.path = path
        self.src = src
        self.rng = rng
        self.toks, self.comments = tokenize(src)
        self.match, self.parent = pair_brackets(self.toks)
        self._obj = {}
        self._calls = None

    def t(self, i):
        if 0 <= i < len(self.toks):
            return self.toks[i]
        return None

    def tx(self, i):
        t = self.t(i)
        return t.text if t is not None else None

    def is_punct(self, i, *texts):
        t = self.t(i)
        return t is not None and t.kind == "punct" and t.text in texts

    def is_id(self, i, *texts):
        t = self.t(i)
        return t is not None and t.kind == "id" and (not texts or t.text in texts)

    def is_plain_id(self, i):
        t = self.t(i)
        return t is not None and t.kind == "id" and t.text not in KEYWORDS and not t.text.startswith("#")

    def is_type_alias_eq(self, i):
        # i is '=': is this `type X =` / `type X<...> =`?
        j = i - 1
        if self.is_punct(j, ">"):
            depth = 0
            while j >= 0:
                if self.is_punct(j, ">"):
                    depth += 1
                elif self.is_punct(j, "<"):
                    depth -= 1
                    if depth == 0:
                        break
                j -= 1
            j -= 1
        return self.is_plain_id(j) and self.is_id(j - 1, "type")

    def is_obj_literal(self, b):
        """b: index of '{'. Heuristic: an object literal expression (not a block, class body,
        type literal, or destructuring target on the left of '=')."""
        if b in self._obj:
            return self._obj[b]
        r = False
        if self.is_punct(b, "{"):
            p = self.t(b - 1)
            if p is not None:
                if p.text in OBJ_PREV and p.kind in ("punct", "id"):
                    r = not (p.text == "=" and self.is_type_alias_eq(b - 1))
                    if r and p.text == "(":
                        # function parameter destructuring `({ a }: T) =>` / `function f({ a })`
                        close = self.match[b]
                        if self.is_punct(close + 1, ":", "="):
                            r = False
                elif p.text == ":" and p.kind == "punct":
                    par = self.parent[b]
                    r = par >= 0 and self.is_obj_literal(par) and not self.is_punct(b - 3, "?")
                if r and self.is_punct(self.match[b] + 1, "="):
                    r = False  # destructuring assignment
        self._obj[b] = r
        return r

    def calls(self):
        """(open_paren_index, [ (first_tok, last_tok) per argument ])"""
        if self._calls is not None:
            return self._calls
        out = []
        for i, t in enumerate(self.toks):
            if not (t.kind == "punct" and t.text == "("):
                continue
            p = self.t(i - 1)
            if p is None:
                continue
            if p.kind == "id":
                if p.text in NOT_CALLEES or p.text.startswith("#") and False:
                    continue
                if self.is_id(i - 2, "function", "get", "set", "async", "constructor"):
                    continue
            elif not (p.kind == "punct" and p.text in (")", "]", ">")):
                continue
            if p.kind == "punct" and p.text == ">":
                continue
            close = self.match[i]
            nt = self.t(close + 1)
            if nt is not None and nt.kind == "punct" and nt.text in ("{", ":", "=>"):
                continue
            if p.kind == "id" and p.text == "constructor":
                continue
            args = []
            cur = None
            for j in range(i + 1, close):
                if self.parent[j] != i:
                    continue
                if self.is_punct(j, ","):
                    if cur is not None:
                        args.append(cur)
                    cur = None
                    continue
                if cur is None:
                    cur = [j, j]
            # extend last token of each arg to cover nested brackets
            fixed = []
            bounds = [j for j in range(i + 1, close) if self.parent[j] == i and self.is_punct(j, ",")] + [close]
            for a in args + ([cur] if cur is not None else []):
                end = min(b for b in bounds if b > a[0]) - 1
                fixed.append((a[0], end))
            out.append((i, fixed))
        self._calls = out
        return out

    def span(self, a, b):
        return self.toks[a].start, self.toks[b].end

    def text(self, a, b):
        s, e = self.span(a, b)
        return self.src[s:e]

    def stmt_start(self, j):
        """First token of the statement containing j, within j's parent."""
        par = self.parent[j]
        k = j
        while k - 1 > par and not (self.is_punct(k - 1, *STMT_BOUND) and self.parent[k - 1] == par):
            k -= 1
        return k

    def enclosing_fn_open(self, j):
        """Index of the '{' of the nearest enclosing function body, or -1."""
        p = self.parent[j]
        while p >= 0:
            if self.is_punct(p, "{") and not self.is_obj_literal(p):
                q = p - 1
                if self.is_punct(q, "=>"):
                    return p
                # skip return type annotation back to ')'
                k = q
                while k >= 0 and self.parent[k] == self.parent[p] and not self.is_punct(k, ")") and not self.is_punct(k, *STMT_BOUND):
                    k -= 1
                if self.is_punct(k, ")"):
                    return p
            p = self.parent[p]
        return -1

    def fn_name_of_body(self, b):
        k = b - 1
        while k >= 0 and not self.is_punct(k, ")"):
            k -= 1
        if k < 0:
            return None
        o = self.match[k]
        return self.tx(o - 1)


def typo(name, rng):
    """A near-miss spelling of `name` (spelling suggestions stay in reach)."""
    if len(name) >= 5 and rng.random() < 0.45:
        k = rng.randrange(1, len(name) - 1)
        return name[:k] + name[k + 1:]
    if len(name) >= 4 and rng.random() < 0.5:
        k = rng.randrange(1, len(name) - 2)
        if name[k] != name[k + 1]:
            return name[:k] + name[k + 1] + name[k] + name[k + 2:]
    return name + rng.choice(["s", "x", "Id", "2"])


# ---------------------------------------------------------------- mutators


def obj_props(ctx):
    """(key_index, colon_index, value_end_index, brace_index) for `key: value` in object literals."""
    out = []
    for i, t in enumerate(ctx.toks):
        if t.kind != "id" or t.text.startswith("#"):
            continue
        if not ctx.is_punct(i + 1, ":"):
            continue
        b = ctx.parent[i]
        if b < 0 or not ctx.is_punct(b, "{") or not ctx.is_obj_literal(b):
            continue
        if not (ctx.is_punct(i - 1, "{", ",")):
            continue
        close = ctx.match[b]
        j = i + 2
        while j < close and not (ctx.parent[j] == b and ctx.is_punct(j, ",")):
            j += 1
        out.append((i, i + 1, j - 1, b))
    return out


def m_prop_rename(ctx):
    res = []
    for k, _c, _v, _b in obj_props(ctx):
        t = ctx.toks[k]
        new = typo(t.text, ctx.rng)
        res.append(Edit("prop_rename", t.start, t.end, new, f"{t.text} -> {new}"))
    return res


def m_prop_drop(ctx):
    res = []
    for k, _c, v, b in obj_props(ctx):
        close = ctx.match[b]
        s = ctx.toks[k].start
        if ctx.is_punct(v + 1, ",") and v + 1 < close:
            e = ctx.toks[v + 1].end
        elif ctx.is_punct(k - 1, ","):
            s = ctx.toks[k - 1].start
            e = ctx.toks[v].end
        else:
            e = ctx.toks[v].end
        if e - s > 400:
            continue
        res.append(Edit("prop_drop", s, e, "", f"drop {ctx.toks[k].text}"))
    return res


STR_PREV = {":", "===", "!==", "==", "!=", "case", "(", ",", "=", "return", "?", "["}
STR_SKIP_CALLEES = {"import", "require", "mock", "describe", "it", "test", "from", "log", "info", "warn",
                    "error", "debug", "Error", "each", "skip", "only", "todo", "t", "span", "startSpan"}


def m_str_change(ctx):
    res = []
    for i, t in enumerate(ctx.toks):
        if t.kind != "str":
            continue
        body = t.text[1:-1]
        if not body or len(body) > 40 or not all(ch.isalnum() or ch in "_-.:/" for ch in body):
            continue
        p = ctx.t(i - 1)
        if p is None or p.text not in STR_PREV:
            continue
        if ctx.is_id(i - 1, "from") or ctx.is_id(i - 2, "import"):
            continue
        if ctx.is_punct(i + 1, ":") and ctx.is_punct(ctx.parent[i], "{"):
            continue
        par = ctx.parent[i]
        if par >= 0 and ctx.is_punct(par, "(") and ctx.tx(par - 1) in STR_SKIP_CALLEES:
            continue
        new = t.text[0] + body + "x" + t.text[-1]
        res.append(Edit("str_change", t.start, t.end, new, f"{t.text} -> {new}"))
    return res


def short_args(ctx, args, limit=80):
    return all(ctx.toks[b].end - ctx.toks[a].start <= limit for a, b in args)


def m_swap_args(ctx):
    res = []
    for o, args in ctx.calls():
        if len(args) < 2 or not short_args(ctx, args):
            continue
        k = ctx.rng.randrange(len(args) - 1)
        a, b = args[k], args[k + 1]
        ta, tb = ctx.text(*a), ctx.text(*b)
        if ta == tb:
            continue
        s, _ = ctx.span(*a)
        _, e = ctx.span(*b)
        between = ctx.src[ctx.toks[a[1]].end:ctx.toks[b[0]].start]
        res.append(Edit("swap_args", s, e, tb + between + ta, f"swap args {k},{k + 1} of {ctx.tx(o - 1)}"))
    return res


def m_remove_arg(ctx):
    res = []
    for o, args in ctx.calls():
        if not args:
            continue
        k = ctx.rng.randrange(len(args))
        a = args[k]
        s, e = ctx.span(*a)
        if e - s > 400:
            continue
        if len(args) > 1:
            if k > 0:
                s = ctx.toks[args[k - 1][1]].end
            else:
                e = ctx.toks[args[1][0]].start
        res.append(Edit("remove_arg", s, e, "", f"remove arg {k} of {ctx.tx(o - 1)}"))
    return res


def m_add_arg(ctx):
    res = []
    for o, args in ctx.calls():
        close = ctx.match[o]
        if args:
            pos = ctx.toks[args[-1][1]].end
            repl = ", 42"
        else:
            pos = ctx.toks[close].start
            repl = "42"
        res.append(Edit("add_arg", pos, pos, repl, f"extra arg to {ctx.tx(o - 1)}"))
    return res


def m_remove_await(ctx):
    res = []
    for i, t in enumerate(ctx.toks):
        if t.kind == "id" and t.text == "await" and not ctx.is_id(i - 1, "for") and not ctx.is_id(i + 1, "using"):
            nt = ctx.t(i + 1)
            if nt is None or nt.kind == "punct" and nt.text in (")", ";", ",", "}", "]"):
                continue
            res.append(Edit("remove_await", t.start, nt.start, "", "remove await"))
    return res


NUM_PREV = {":", "=", "(", ",", "return", "===", "!==", "<", ">", "+", "-", "*", "[", "?", "??", "||"}


def m_num_to_str(ctx):
    res = []
    for i, t in enumerate(ctx.toks):
        if t.kind == "num" and ctx.tx(i - 1) in NUM_PREV and not ctx.is_punct(i + 1, ":"):
            res.append(Edit("num_to_str", t.start, t.end, f"'{t.text}'", f"{t.text} -> '{t.text}'"))
    return res


def m_delete_return(ctx):
    res = []
    for i, t in enumerate(ctx.toks):
        if t.kind == "id" and t.text == "return":
            nt = ctx.t(i + 1)
            if nt is None or nt.nl_before or nt.kind == "punct" and nt.text in (";", "}"):
                continue
            res.append(Edit("delete_return", t.start, nt.start, "", "delete return"))
    return res


def imported_names(ctx):
    names = []
    toks = ctx.toks
    for i, t in enumerate(toks):
        if not (t.kind == "id" and t.text == "import" and (t.nl_before or i == 0)):
            continue
        j = i + 1
        if ctx.is_id(j, "type"):
            continue
        # default import
        if ctx.is_plain_id(j) and (ctx.is_punct(j + 1, ",") or ctx.is_id(j + 1, "from")):
            names.append(toks[j].text)
            j += 2 if ctx.is_punct(j + 1, ",") else 1
        if ctx.is_punct(j, "*") and ctx.is_id(j + 1, "as") and ctx.is_plain_id(j + 2):
            names.append(toks[j + 2].text)
        elif ctx.is_punct(j, "{"):
            close = ctx.match[j]
            k = j + 1
            while k < close:
                # element: [type] name [as alias]
                s = k
                while k < close and not ctx.is_punct(k, ","):
                    k += 1
                elt = [x for x in range(s, k)]
                if elt and not ctx.is_id(elt[0], "type"):
                    last = elt[-1]
                    if ctx.is_plain_id(last) or ctx.toks[last].kind == "id":
                        names.append(toks[last].text)
                k += 1
    return set(names)


def m_rename_import_use(ctx):
    names = imported_names(ctx)
    res = []
    if not names:
        return res
    for i, t in enumerate(ctx.toks):
        if t.kind != "id" or t.text not in names:
            continue
        if ctx.is_punct(i - 1, ".", "?.") or ctx.is_id(i - 1, "import", "as", "type") :
            continue
        # skip the import declarations themselves
        k = ctx.stmt_start(i)
        if ctx.is_id(k, "import", "export"):
            continue
        if ctx.is_punct(i + 1, ":") and ctx.is_punct(ctx.parent[i], "{"):
            continue
        new = typo(t.text, ctx.rng)
        res.append(Edit("rename_import_use", t.start, t.end, new, f"{t.text} -> {new}"))
    return res


def m_eq_incompat(ctx):
    res = []
    for i, t in enumerate(ctx.toks):
        if not (t.kind == "punct" and t.text in ("===", "!==")):
            continue
        r = ctx.t(i + 1)
        if r is None:
            continue
        nt = ctx.t(i + 2)
        if nt is not None and nt.kind == "punct" and nt.text in (".", "(", "[", "?.", "!"):
            continue
        if r.kind == "str":
            new = "12345"
        elif r.kind == "num":
            new = "'zz'"
        elif r.kind == "id" and r.text in ("true", "false"):
            new = "'yes'"
        else:
            continue
        res.append(Edit("eq_incompat", r.start, r.end, new, f"{r.text} -> {new}"))
    return res


TYPE_STOP = {",", ")", ";", "]", "}", "=", "?", ":", "&&", "||", "??", "=>", "+", "-", "*", "/", "==", "===", "!=", "!=="}


def type_extent(ctx, j):
    """Index of the last token of a type starting at token j (heuristic)."""
    k = j
    last = j - 1
    n = len(ctx.toks)
    while k < n:
        t = ctx.toks[k]
        if k > j and t.nl_before and not ctx.is_punct(last, "|", "&", "<", ",", "."):
            if not (t.kind == "punct" and t.text in ("|", "&", ".", "<", "[")):
                break
        if t.kind == "punct":
            if t.text in ("(", "[", "{"):
                last = ctx.match[k]
                k = last + 1
                continue
            if t.text == "<":
                depth = 0
                m = k
                while m < n:
                    if ctx.is_punct(m, "<"):
                        depth += 1
                    elif ctx.is_punct(m, ">"):
                        depth -= 1
                        if depth == 0:
                            break
                    elif ctx.is_punct(m, "(", "[", "{"):
                        m = ctx.match[m]
                    elif ctx.is_punct(m, ";", ")", "}", "]"):
                        return last
                    m += 1
                last = m
                k = m + 1
                continue
            if t.text in (".", "|", "&"):
                last = k
                k += 1
                continue
            break
        if t.kind == "id" and t.text in ("as", "satisfies", "in", "of", "extends", "instanceof") and k > j:
            break
        if t.kind == "id" and k > j and last >= 0 and ctx.toks[last].kind == "id" and ctx.toks[last].text not in ("keyof", "typeof", "readonly", "unique"):
            break
        last = k
        k += 1
    return last


def m_remove_as(ctx):
    res = []
    for i, t in enumerate(ctx.toks):
        if not (t.kind == "id" and t.text == "as"):
            continue
        if ctx.is_punct(i - 1, "*") or ctx.is_punct(ctx.parent[i], "{") and ctx.is_id(ctx.parent[i] - 1, "import", "export"):
            continue
        p = ctx.t(i - 1)
        if p is None or p.kind == "punct" and p.text in ("{", ",", "(") :
            continue
        k = ctx.stmt_start(i)
        if ctx.is_id(k, "import", "export") and ctx.is_punct(ctx.parent[i], "{"):
            continue
        end = type_extent(ctx, i + 1)
        if end <= i:
            continue
        s = p.end
        e = ctx.toks[end].end
        res.append(Edit("remove_as", s, e, "", "remove" + ctx.src[s:e][:60]))
    return res


def generic_lists(ctx):
    """(lt, gt, [arg (first,last)...]) for `Name<...>` with no space before '<'."""
    out = []
    n = len(ctx.toks)
    for i, t in enumerate(ctx.toks):
        if not (t.kind == "punct" and t.text == "<"):
            continue
        p = ctx.t(i - 1)
        if p is None or p.kind != "id" or p.end != t.start or p.text in KEYWORDS:
            continue
        depth = 0
        m = i
        ok = False
        while m < n and ctx.toks[m].start - t.start < 400:
            if ctx.is_punct(m, "<"):
                depth += 1
            elif ctx.is_punct(m, ">"):
                depth -= 1
                if depth == 0:
                    ok = True
                    break
            elif ctx.is_punct(m, "(", "[", "{"):
                m = ctx.match[m]
            elif ctx.is_punct(m, ";", ")", "}", "]", "&&", "||", "="):
                break
            m += 1
        if not ok:
            continue
        args = []
        cur = i + 1
        d = 0
        for k in range(i + 1, m + 1):
            if ctx.is_punct(k, "<"):
                d += 1
            elif ctx.is_punct(k, ">"):
                if d == 0:
                    args.append((cur, k - 1))
                    break
                d -= 1
            elif ctx.is_punct(k, "(", "[", "{") and ctx.match[k] > k:
                pass
            if ctx.parent[k] != ctx.parent[i]:
                continue
            if d == 0 and ctx.is_punct(k, ","):
                args.append((cur, k - 1))
                cur = k + 1
        out.append((i, m, args))
    return out


def m_generic_arg(ctx):
    res = []
    for lt, gt, args in generic_lists(ctx):
        cands = [(a, b) for a, b in args if a == b and ctx.toks[a].kind == "id"]
        if not cands:
            continue
        a, _ = ctx.rng.choice(cands)
        t = ctx.toks[a]
        new = {"string": "number", "number": "string", "boolean": "string"}.get(t.text, "number")
        res.append(Edit("generic_arg", t.start, t.end, new, f"{ctx.tx(lt - 1)}<{t.text}> -> <{new}>"))
    return res


def m_drop_type_args(ctx):
    """`f<T>(...)` -> `f(...)`: the call now infers its type arguments."""
    res = []
    for lt, gt, _args in generic_lists(ctx):
        if not ctx.is_punct(gt + 1, "("):
            continue
        s, e = ctx.toks[lt].start, ctx.toks[gt].end
        res.append(Edit("drop_type_args", s, e, "", f"{ctx.tx(lt - 1)}{ctx.src[s:e][:60]}( -> {ctx.tx(lt - 1)}("))
    return res


def m_remove_nullish(ctx):
    """`a ?? b` -> `a` for a one-token (or empty bracket pair) right operand."""
    res = []
    for i, t in enumerate(ctx.toks):
        if not (t.kind == "punct" and t.text == "??"):
            continue
        j = i + 1
        nt = ctx.t(j)
        if ctx.is_punct(j, "[", "{", "(") and ctx.match[j] == j + 1:
            last = j + 1
        elif nt is not None and nt.kind in ("id", "str", "num") and not ctx.is_punct(j + 1, ".", "(", "[", "?.", "<"):
            last = j
        else:
            continue
        after = ctx.t(last + 1)
        if not ctx.is_punct(last + 1, ",", ";", ")", "}", "]") and not (after is not None and after.nl_before):
            continue
        res.append(Edit("remove_nullish", ctx.toks[i - 1].end, ctx.toks[last].end, "", f"drop ?? {ctx.text(j, last)}"))
    return res


def m_flip_optchain(ctx):
    res = []
    for i, t in enumerate(ctx.toks):
        if t.kind == "punct" and t.text == "?.":
            nt = ctx.t(i + 1)
            if nt is not None and nt.kind == "punct" and nt.text in ("(", "["):
                res.append(Edit("flip_optchain", t.start, t.end, "", "?.( -> ("))
            else:
                res.append(Edit("flip_optchain", t.start, t.end, ".", "?. -> ."))
        elif t.kind == "punct" and t.text == "!":
            p = ctx.t(i - 1)
            nt = ctx.t(i + 1)
            if p is None or nt is None or p.end != t.start:
                continue
            if not (p.kind == "id" or p.kind == "punct" and p.text in (")", "]")):
                continue
            if p.kind == "id" and p.text in KEYWORDS and p.text != "this":
                continue
            if nt.kind == "punct" and nt.text in (".", "[", ")", ",", ";", "?."):
                # skip definite assignment `x!: T` (never followed by these) - these are non-null assertions
                res.append(Edit("flip_optchain", t.start, t.end, "", "remove non-null !"))
    # rare reverse direction: a.b -> a?.b (only on member calls)
    for i, t in enumerate(ctx.toks):
        if t.kind == "punct" and t.text == "." and ctx.rng.random() < 0.05:
            p = ctx.t(i - 1)
            if p is not None and (p.kind == "id" and p.text not in KEYWORDS or p.text in (")", "]")) and ctx.is_plain_id(i + 1):
                res.append(Edit("flip_optchain", t.start, t.end, "?.", ". -> ?."))
    return res


def m_const_assign(ctx):
    res = []
    for i, t in enumerate(ctx.toks):
        if not (t.kind == "id" and t.text == "const" and ctx.is_plain_id(i + 1) and ctx.is_punct(i + 2, "=", ":")):
            continue
        name = ctx.toks[i + 1].text
        par = ctx.parent[i]
        if par < 0 or ctx.enclosing_fn_open(i) != par:
            continue
        close = ctx.match[par]
        for j in range(i + 3, close):
            if ctx.parent[j] == par and ctx.is_id(j, "return"):
                r = ctx.toks[j]
                res.append(Edit("const_assign", r.start, r.start, f"{name} = {name}; ", f"assign const {name}"))
                break
    # readonly members: this.x = this.x; before a statement that uses this.x in a non-constructor method
    ro = set()
    for i, t in enumerate(ctx.toks):
        if t.kind == "id" and t.text == "readonly" and ctx.is_plain_id(i + 1) and ctx.is_punct(i + 2, ":", "!", "=", "?"):
            ro.add(ctx.toks[i + 1].text)
    if ro:
        for j, t in enumerate(ctx.toks):
            if t.kind == "id" and t.text == "this" and ctx.is_punct(j + 1, ".") and ctx.is_id(j + 2) and ctx.toks[j + 2].text in ro:
                fb = ctx.enclosing_fn_open(j)
                if fb < 0 or ctx.fn_name_of_body(fb) == "constructor":
                    continue
                s = ctx.stmt_start(j)
                if not ctx.is_punct(ctx.parent[s], "{") or ctx.is_obj_literal(ctx.parent[s]):
                    continue
                name = ctx.toks[j + 2].text
                pos = ctx.toks[s].start
                res.append(Edit("const_assign", pos, pos, f"this.{name} = this.{name}; ", f"assign readonly {name}"))
    return res


def m_remove_case(ctx):
    res = []
    for i, t in enumerate(ctx.toks):
        if not (t.kind == "id" and t.text == "case"):
            continue
        par = ctx.parent[i]
        if par < 0 or not ctx.is_punct(par, "{"):
            continue
        close = ctx.match[par]
        j = i + 1
        while j < close and not (ctx.parent[j] == par and ctx.is_id(j, "case", "default")):
            j += 1
        s = t.start
        e = ctx.toks[j].start if j < close else ctx.toks[close].start
        if e - s > 3000:
            continue
        res.append(Edit("remove_case", s, e, "", "remove " + ctx.src[s:ctx.toks[i + 1].end]))
    return res


def m_bad_method(ctx):
    res = []
    for i, t in enumerate(ctx.toks):
        if not (t.kind == "punct" and t.text in (".", "?.")):
            continue
        p = ctx.t(i - 1)
        if p is None or not (p.kind == "id" or p.kind == "punct" and p.text in (")", "]")):
            continue
        if not ctx.is_id(i + 1) or ctx.toks[i + 1].text.startswith("#"):
            continue
        if ctx.is_id(ctx.stmt_start(i), "import", "export"):
            continue
        m = ctx.toks[i + 1]
        if ctx.rng.random() < 0.8:
            new = typo(m.text, ctx.rng)
        else:
            new = "notAMember" + str(ctx.rng.randrange(100))
        res.append(Edit("bad_method", m.start, m.end, new, f".{m.text} -> .{new}"))
    return res


def class_like_bodies(ctx):
    """(kind, open_brace) for class bodies with extends/implements and interfaces with extends."""
    out = []
    for i, t in enumerate(ctx.toks):
        if t.kind != "id" or t.text not in ("class", "interface"):
            continue
        j = i + 1
        seen = False
        while j < len(ctx.toks) and not ctx.is_punct(j, "{"):
            if ctx.is_id(j, "extends", "implements"):
                seen = True
            if ctx.is_punct(j, ";", "(", ")", "=", "}"):
                break
            if ctx.is_punct(j, "<"):
                pass
            j += 1
        if seen and ctx.is_punct(j, "{"):
            out.append((t.text, j))
    return out


def m_member_sig(ctx):
    res = []
    for kind, b in class_like_bodies(ctx):
        close = ctx.match[b]
        for i in range(b + 1, close):
            if ctx.parent[i] != b or not ctx.is_id(i):
                continue
            if not ctx.is_punct(i - 1, "{", ";", "}") and not ctx.toks[i].nl_before:
                continue
            k = i
            while ctx.is_id(k, "public", "private", "protected", "static", "async", "readonly", "override", "abstract"):
                k += 1
            if not ctx.is_plain_id(k) and not ctx.is_id(k):
                continue
            q = k + 1
            if ctx.is_punct(q, "?"):
                q += 1
            if ctx.is_punct(q, "("):
                cp = ctx.match[q]
                # first annotated param
                opts = []
                for j in range(q + 1, cp):
                    if ctx.parent[j] == q and ctx.is_punct(j, ":"):
                        end = type_extent(ctx, j + 1)
                        if end > j:
                            opts.append((j + 1, end, "param"))
                        break
                if ctx.is_punct(cp + 1, ":"):
                    end = type_extent(ctx, cp + 2)
                    if end > cp + 1:
                        opts.append((cp + 2, end, "return"))
                if opts:
                    a, e, what = ctx.rng.choice(opts)
                    s, en = ctx.span(a, e)
                    res.append(Edit("member_sig", s, en, "symbol", f"{ctx.tx(k)} {what} {ctx.src[s:en][:50]} -> symbol"))
            elif kind == "interface" and ctx.is_punct(q, ":"):
                end = type_extent(ctx, q + 1)
                if end > q:
                    s, en = ctx.span(q + 1, end)
                    res.append(Edit("member_sig", s, en, "symbol", f"{ctx.tx(k)}: {ctx.src[s:en][:50]} -> symbol"))
    return res


def m_zod_swap(ctx):
    res = []
    swap = {"string": "number", "number": "string", "boolean": "string", "date": "string", "uuid": "number"}
    for i, t in enumerate(ctx.toks):
        if t.kind == "id" and t.text == "z" and ctx.is_punct(i + 1, ".") and ctx.is_id(i + 2) and ctx.toks[i + 2].text in swap and ctx.is_punct(i + 3, "("):
            m = ctx.toks[i + 2]
            res.append(Edit("zod_swap", m.start, m.end, swap[m.text], f"z.{m.text} -> z.{swap[m.text]}"))
        if t.kind == "punct" and t.text == "." and ctx.is_id(i + 1, "optional", "nullable", "nullish") and ctx.is_punct(i + 2, "(") and ctx.is_punct(i + 3, ")"):
            p = ctx.t(i - 1)
            if p is not None and p.kind == "punct" and p.text == ")":
                res.append(Edit("zod_swap", t.start, ctx.toks[i + 3].end, "", f"remove .{ctx.tx(i + 1)}()"))
    return res


SIMPLE_TYPES = {"string": "number", "number": "string", "boolean": "string", "Date": "string", "bigint": "string"}


def type_member_braces(ctx):
    """Braces that are type literals / interface bodies / class bodies."""
    out = {}
    for i, t in enumerate(ctx.toks):
        if not (t.kind == "punct" and t.text == "{"):
            continue
        if ctx.is_obj_literal(i):
            continue
        # walk back to statement start
        s = ctx.stmt_start(i)
        k = s
        while ctx.is_id(k, "export", "default", "declare", "abstract"):
            k += 1
        if ctx.is_id(k, "interface"):
            out[i] = "interface"
        elif ctx.is_id(k, "class"):
            out[i] = "class"
        elif ctx.is_id(k, "type") and ctx.is_punct(i - 1, "=", "&", "|", "<", ","):
            out[i] = "type"
        elif ctx.is_punct(i - 1, ":") and not ctx.is_punct(ctx.parent[i], "{"):
            out[i] = "type"  # annotation type literal, e.g. param: { a: string }
    return out


def m_prop_type_change(ctx):
    res = []
    braces = type_member_braces(ctx)
    for i, t in enumerate(ctx.toks):
        if t.kind != "id" or ctx.parent[i] not in braces:
            continue
        if not ctx.is_punct(i - 1, "{", ";", ",", "}") and not t.nl_before and not ctx.is_id(i - 1, "readonly"):
            continue
        q = i + 1
        if ctx.is_punct(q, "?", "!"):
            q += 1
        if not ctx.is_punct(q, ":"):
            continue
        ty = ctx.t(q + 1)
        if ty is None or ty.kind != "id" or ty.text not in SIMPLE_TYPES:
            continue
        nt = ctx.t(q + 2)
        if nt is not None and not (nt.nl_before or nt.kind == "punct" and nt.text in (";", ",", "}", "|", "=")):
            continue
        new = SIMPLE_TYPES[ty.text]
        res.append(Edit("prop_type_change", ty.start, ty.end, new, f"{t.text}: {ty.text} -> {new}"))
    return res


def m_optional_toggle(ctx):
    res = []
    braces = type_member_braces(ctx)
    for i, t in enumerate(ctx.toks):
        if t.kind != "id" or ctx.parent[i] not in braces:
            continue
        if not ctx.is_punct(i - 1, "{", ";", ",", "}") and not t.nl_before and not ctx.is_id(i - 1, "readonly"):
            continue
        kind = braces[ctx.parent[i]]
        if ctx.is_punct(i + 1, "?") and ctx.is_punct(i + 2, ":"):
            q = ctx.toks[i + 1]
            res.append(Edit("optional_toggle", q.start, q.end, "!" if kind == "class" else "", f"{t.text}? -> required"))
        elif ctx.is_punct(i + 1, ":") and kind != "class":
            res.append(Edit("optional_toggle", t.end, t.end, "?", f"{t.text} -> {t.text}?"))
        elif ctx.is_punct(i + 1, "!") and ctx.is_punct(i + 2, ":"):
            q = ctx.toks[i + 1]
            res.append(Edit("optional_toggle", q.start, q.end, "?", f"{t.text}! -> {t.text}?"))
    return res


def m_remove_async(ctx):
    res = []
    for i, t in enumerate(ctx.toks):
        if t.kind == "id" and t.text == "async":
            nt = ctx.t(i + 1)
            if nt is None or nt.nl_before:
                continue
            if nt.kind == "id" or nt.kind == "punct" and nt.text in ("(", "<", "*"):
                res.append(Edit("remove_async", t.start, nt.start, "", "remove async"))
    return res


MUTATORS = {
    "prop_rename": m_prop_rename,
    "prop_drop": m_prop_drop,
    "str_change": m_str_change,
    "swap_args": m_swap_args,
    "remove_arg": m_remove_arg,
    "add_arg": m_add_arg,
    "remove_await": m_remove_await,
    "num_to_str": m_num_to_str,
    "delete_return": m_delete_return,
    "rename_import_use": m_rename_import_use,
    "eq_incompat": m_eq_incompat,
    "remove_as": m_remove_as,
    "generic_arg": m_generic_arg,
    "flip_optchain": m_flip_optchain,
    "const_assign": m_const_assign,
    "remove_case": m_remove_case,
    "bad_method": m_bad_method,
    "member_sig": m_member_sig,
    "zod_swap": m_zod_swap,
    "prop_type_change": m_prop_type_change,
    "optional_toggle": m_optional_toggle,
    "remove_async": m_remove_async,
    "drop_type_args": m_drop_type_args,
    "remove_nullish": m_remove_nullish,
}


def pick_mutation(path, src, rng, weights=None):
    """Chooses one mutation for the file: a mutator (among those with candidates) then a site."""
    ctx = Ctx(path, src, rng)
    avail = {}
    for name, fn in MUTATORS.items():
        cands = fn(ctx)
        if cands:
            avail[name] = cands
    if not avail:
        return None
    names = sorted(avail)
    w = [(weights or {}).get(n, 1.0) for n in names]
    name = rng.choices(names, weights=w)[0]
    e = rng.choice(avail[name])
    return e
