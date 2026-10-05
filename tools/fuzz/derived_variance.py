#!/usr/bin/env python3
"""Adversarial programs for TSRS_DERIVED_VARIANCE (notes/fuzz-derived-variance.md, notes/perf-derived-variance.md).

  tools/fuzz/derived_variance.py gen  --seed S [--out DIR]          # write one program (a.ts + tsconfig.json)
  tools/fuzz/derived_variance.py run  [--start S] [--count N] [--jobs J] [--out DIR] [--tsrs PATH]
                                      [--exclude f1,f2] [--min-members N] [--onoff-every K]

Each program (deterministic from its seed) declares a generic base class or interface with 16+ members that use its
type parameters and `this` in the positions the variance machinery treats specially, derived classes / interfaces
that pass the arguments through, fix, permute or wrap them and add or override members, and relation sites that
relate derived instances to base references with argument pairs around the edges (identical, sub/supertypes,
unrelated, any, unknown, never, {}, unions, literals, optional, enums, unique symbols, type parameters inside generic
functions) under one of several option sets.

`run` type-checks each program with TSRS_DERIVED_VARIANCE=shadow (a disagreement = exit 7 + a report on stderr) and,
every K-th program (default 1), compares `on` against `off` byte for byte. Members are named `<feature>_<n>`, so the
culprit list of a disagreement names the features involved; findings are saved under --out/<seed>/ with the shadow
report. Prints a summary: programs, decisions, disagreements by culprit feature, decisions per feature (a decision is
credited to every feature of the program's base). `--exclude` drops features from the generator (to search for new
kinds of disagreement once the known ones are understood). Exit status 1 if any finding.
"""

import argparse
import collections
import concurrent.futures
import json
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TSRS = os.path.join(ROOT, "target", "release", "tsrs")
TIMEOUT = 60

OPTION_SETS = [
    ("strict", {"strict": True}),
    ("strict", {"strict": True}),
    ("strict-eopt", {"strict": True, "exactOptionalPropertyTypes": True}),
    ("strict-nosft", {"strict": True, "strictFunctionTypes": False}),
    ("nostrictnull", {"strict": True, "strictNullChecks": False}),
    ("loose", {}),
]

# Type-argument pools. Each entry: (text, kind). Pairs are chosen around a small lattice.
OBJ = ["{}", "{ a?: string }", "{ a: string }", "{ a: string; b: number }", "{ readonly a: string }", "{ a: 1 }",
       "{ a?: string | undefined }", "{ b?: number }", "object", "{ [k: string]: string }", "{ a: string } & { c: 1 }"]
PRIM = ["string", "number", "boolean", "\"a\"", "\"b\"", "\"a\" | \"b\"", "1", "0", "string | number", "true",
        "bigint", "symbol", "`x${string}`", "Lowercase<string>", "string & {}"]
SPECIAL = ["any", "unknown", "never", "undefined", "null", "void", "string | undefined", "E", "E.A", "E.B", "S1",
           "typeof sym", "symbol", "Box<string>", "Box<any>", "Box<never>", "string[]", "readonly string[]",
           "[string]", "[string, number?]", "() => void", "(x: string) => void", "Function"]
STRINGY = ["string", "\"a\"", "\"b\"", "\"a\" | \"b\"", "`x${string}`", "any", "never", "Lowercase<string>",
           "string & {}", "S1", "\"a\" | \"c\""]

# Directed edges of the lattice: a -> b means a is (meant to be) a subtype of b.
SUB = [("\"a\"", "string"), ("\"a\"", "\"a\" | \"b\""), ("\"a\" | \"b\"", "string"), ("string", "string | number"),
       ("1", "number"), ("0", "number"), ("true", "boolean"), ("never", "string"), ("string", "unknown"),
       ("{ a: string; b: number }", "{ a: string }"), ("{ a: string }", "{}"), ("{ a: string }", "{ a?: string }"),
       ("{ a?: string }", "{}"), ("{}", "{ a?: string }"), ("{}", "{ b?: number }"), ("{ readonly a: string }", "{ a: string }"),
       ("{ a: string }", "{ readonly a: string }"), ("{ a: 1 }", "{ a: string }"), ("E.A", "E"), ("E", "number"),
       ("0", "E"), ("typeof sym", "symbol"), ("S1", "string"), ("string", "{}"), ("{ a: string }", "object"),
       ("string[]", "readonly string[]"), ("[string]", "string[]"), ("[string]", "[string, number?]"),
       ("Box<\"a\">", "Box<string>"), ("Box<never>", "Box<string>"), ("{ a: string } & { c: 1 }", "{ a: string }"),
       ("{ a: string }", "{ [k: string]: string }"), ("`x${string}`", "string"), ("string & {}", "string"),
       ("string", "string & {}"), ("Lowercase<string>", "string"), ("undefined", "string | undefined"),
       ("string", "string | undefined"), ("{ a?: string | undefined }", "{ a?: string }"),
       ("{ a?: string }", "{ a?: string | undefined }"), ("(x: string) => void", "() => void"),
       ("() => void", "(x: string) => void"), ("() => void", "Function"), ("null", "string | null"),
       ("\"a\"", "\"a\" | \"c\""), ("void", "undefined"), ("undefined", "void")]

FEATURES = [
    # name, template (X = a type parameter, Y = another one, B = the base's name, P = its parameter list), flags
    ("prop", "{n}: X;", ""),
    ("readonly", "readonly {n}: X;", ""),
    ("optional", "{n}?: X;", ""),
    ("method_param", "{n}(x: X): void;", ""),
    ("method_ret", "{n}(): X;", ""),
    ("method_both", "{n}(x: X): X;", ""),
    ("fnprop_param", "{n}: (x: X) => void;", ""),
    ("fnprop_ret", "{n}: () => X;", ""),
    ("fnprop_both", "{n}: (x: X) => X;", ""),
    ("union_undef", "{n}: X | undefined | null;", ""),
    ("intersection", "{n}: X & {{ z?: 1 }};", ""),
    ("array", "{n}: X[];", ""),
    ("readonly_array", "{n}: readonly X[];", ""),
    ("promise", "{n}: Promise<X>;", ""),
    ("box", "{n}: Box<X>;", ""),
    ("cond_dist", "{n}: X extends string ? 1 : 2;", ""),
    ("cond_nondist", "{n}: [X] extends [string] ? 1 : 2;", ""),
    ("cond_infer", "{n}: X extends {{ a: infer R }} ? R : never;", ""),
    ("cond_ext", "{n}: {{ a: 1 }} extends X ? 1 : 2;", ""),
    ("cond_alias", "{n}: IsStr<X>;", ""),
    ("cond_branch", "{n}: Y extends string ? X : Y;", "2"),
    ("cond_fn", "{n}(x: X extends object ? keyof X : X): void;", ""),
    ("mapped_homo", "{n}: {{ [K in keyof X]: X[K] }};", ""),
    ("mapped_minus", "{n}: {{ -readonly [K in keyof X]-?: X[K] }};", ""),
    ("mapped_opt", "{n}: {{ readonly [K in keyof X]?: 1 }};", ""),
    ("mapped_remap", "{n}: {{ [K in keyof X & string as `get${{K}}`]: () => X[K] }};", ""),
    ("partial", "{n}: Partial<X>;", ""),
    ("record", "{n}: Record<string, X>;", ""),
    ("pick_keys", "{n}: Pick<{{ a: X; b: 1 }}, \"a\">;", ""),
    ("keyof", "{n}: keyof X;", ""),
    ("keyof_param", "{n}(k: keyof X): void;", ""),
    ("keyof_fnparam", "{n}: (k: keyof X) => void;", ""),
    ("indexed", "{n}: X[keyof X];", ""),
    ("indexed_key", "{n}: {{ a: 1; b: 2 }}[X & (\"a\" | \"b\")];", ""),
    ("template", "{n}: `p${{X & string}}`;", ""),
    ("uppercase", "{n}: Uppercase<X & string>;", ""),
    ("tuple", "{n}: [X, ...X[]];", ""),
    ("tuple_opt", "{n}: readonly [X?];", ""),
    ("rest_param", "{n}(...a: X[]): void;", ""),
    ("rest_tuple", "{n}(...a: [X, string?]): void;", ""),
    ("recursive", "{n}: B<P>;", ""),
    ("recursive_wrap", "{n}?: B<PW>;", ""),
    ("this_prop", "{n}: this;", "t"),
    ("this_ret", "{n}(): this;", "t"),
    ("this_param", "{n}(x: this): void;", "t"),
    ("this_fnprop", "{n}: (x: this) => void;", "t"),
    ("this_cond", "{n}: this extends {{ tag: X }} ? 1 : 2;", "t"),
    ("this_keyof", "{n}: keyof this;", "t"),
    ("this_indexed", "{n}(): this[\"{first}\"];", "t"),
    ("this_box", "{n}: Box<this>;", "t"),
    ("overload", "{n}(x: X): X;\n  {n}(x: string, y: number): string;", "o"),
    ("generic_method", "{n}<Q extends X>(q: Q): Q;", ""),
    ("generic_method_this", "{n}<Q>(f: (t: this, x: X) => Q): Q;", "t"),
    ("noinfer", "{n}(x: NoInfer<X>): void;", ""),
    ("uniq", "{n}: typeof sym | X;", ""),
    ("enum_union", "{n}: E | X;", ""),
    ("pred", "{n}(x: unknown): x is X;", ""),
    ("ctor_type", "{n}: new (x: X) => X;", ""),
    ("unknown_fn", "{n}: (x: unknown) => X;", ""),
    ("pad", "{n}: number;", ""),
]

CLASS_ONLY = [
    ("accessor", "get {n}(): X {{ return null!; }}\n  set {n}(v: X) {{ }}", "c"),
    ("getter_only", "get {n}(): X {{ return null!; }}", "c"),
    ("private", "private {n}!: X;", "c"),
    ("protected", "protected {n}!: X;", "c"),
    ("hash_private", "#{n}!: X;", "c"),
    ("abstract", "abstract {n}: X;", "a"),
    ("abstract_method", "abstract {n}(x: X): X;", "a"),
]

PREAMBLE = """enum E { A, B }
declare const sym: unique symbol;
type S1 = "s1";
interface Box<T> { value: T }
type IsStr<T> = T extends string ? true : false;
declare function pick<T>(a: T, b: T): T;
type Mixin<C extends abstract new (...a: any[]) => object> = C;
"""


class Gen:
    def __init__(self, seed, exclude=()):
        self.r = random.Random(seed)
        self.seed = seed
        self.exclude = set(exclude)
        self.out = []
        self.features = collections.Counter()
        self.n = 0

    def name(self, feature):
        self.n += 1
        return f"{feature}_{self.n}"

    def pick_args(self, constraint):
        r = self.r
        if constraint == "string":
            return r.choice(STRINGY)
        pool = r.choice([OBJ, PRIM, SPECIAL, OBJ, PRIM])
        return r.choice(pool)

    def pair(self, constraint):
        """A (source argument, target argument) pair around the edges."""
        r = self.r
        k = r.random()
        if constraint == "string":
            a = r.choice(STRINGY)
            b = r.choice(STRINGY) if k < 0.5 else a
            return a, b
        if k < 0.25:
            a = self.pick_args(None)
            return a, a
        if k < 0.6:
            a, b = r.choice(SUB)
            return (a, b) if r.random() < 0.7 else (b, a)
        if k < 0.75:
            return r.choice(["any", "unknown", "never", "{}"]), self.pick_args(None)
        if k < 0.85:
            return self.pick_args(None), r.choice(["any", "unknown", "never", "{}"])
        return self.pick_args(None), self.pick_args(None)

    def program(self):
        r = self.r
        opts_name, opts = r.choice(OPTION_SETS)
        kind = r.choice(["interface", "interface", "class", "abstract class"])
        nparams = r.choice([1, 1, 2, 2, 3])
        params = ["T", "U", "V"][:nparams]
        constraints = {}
        decl_params = []
        for p in params:
            c = None
            text = p
            if r.random() < 0.15:
                c = "string"
                text += " extends string"
            if r.random() < 0.15:
                text = r.choice(["in ", "out ", "in out "]) + text
            if r.random() < 0.2:
                text += " = " + ("\"a\"" if c == "string" else r.choice(["unknown", "{}", "string", "any"]))
            constraints[p] = c
            decl_params.append(text)
        base = "B" + str(self.seed % 1000)
        features = [f for f in FEATURES if f[0] not in self.exclude]
        if kind != "interface":
            features += [f for f in CLASS_ONLY if f[0] not in self.exclude and (f[2] != "a" or kind == "abstract class")]
        nmembers = r.randint(16, 26)
        # Bias: the members most likely to make a disagreement visible are interesting ones; pads keep 16+ props.
        chosen = [r.choice(features) for _ in range(nmembers)]
        members = []
        first = None
        has_index = r.random() < 0.15 and "index" not in self.exclude
        has_tag = False
        for fname, tmpl, flags in chosen:
            n = self.name(fname)
            X = r.choice(params)
            Y = r.choice(params)
            if constraints[X] == "string" and fname in ("cond_infer", "mapped_homo", "mapped_minus"):
                pass
            P = ", ".join(params)
            PW = ", ".join(f"{p}[]" if constraints[p] is None else p for p in params)
            if fname == "this_indexed" and first is None:
                fname, tmpl, flags = "this_ret", "{n}(): this;", "t"
            text = tmpl.format(n=n, first=first or "")
            text = text.replace("X", X).replace("Y", Y).replace("B<P>", f"{base}<{P}>").replace("B<PW>", f"{base}<{PW}>")
            if kind != "interface" and "c" not in flags and "a" not in flags:
                text = self.classify(text, n)
            if first is None and fname in ("prop", "readonly", "pad"):
                first = n
            members.append(text)
            self.features[fname] += 1
            has_tag |= fname == "this_cond"
        if has_index:
            X = r.choice(params)
            members.append(f"[k: number]: {X};" if kind == "interface" else f"[k: number]: {X};")
            self.features["index"] += 1
        if kind != "interface" and r.random() < 0.3:
            members.append("static s_count: number = 0;")
            self.features["static"] += 1
        while len(members) < 16:
            members.append(self.classify(f"{self.name('pad')}: number;", None) if kind != "interface" else f"{self.name('pad')}: number;")
        out = [PREAMBLE]
        dp = ", ".join(decl_params)
        if kind == "interface":
            out.append(f"interface {base}<{dp}> {{\n  " + "\n  ".join(members) + "\n}")
            if r.random() < 0.2:
                # Interface merging: a second declaration of the base.
                X = r.choice(params)
                out.append(f"interface {base}<{dp}> {{ {self.name('merged')}: {X}; }}")
                self.features["merged_base"] += 1
        else:
            out.append(f"declare {kind} {base}<{dp}> {{\n  " + "\n  ".join(m.replace("!:", ":") for m in members if not m.startswith("static")) + "\n}")
            out[-1] = out[-1].replace("declare abstract class", "declare abstract class")
        self.base = base
        self.params = params
        self.constraints = constraints
        self.kind = kind
        self.has_tag = has_tag
        derived = self.derived(out)
        self.sites(out, derived)
        self.opts_name = opts_name
        cfg = {"compilerOptions": dict(opts, noEmit=True, target="es2022", types=[], lib=["es2022"]), "files": ["a.ts"]}
        return "\n".join(out) + "\n", json.dumps(cfg)

    def classify(self, text, n):
        # Interface member syntax -> declare class member syntax (bodies are not allowed in declare classes).
        return text

    def derived(self, out):
        """Derived types: [(name, nparams, constraints-of-own-params, fn(own args) -> base args text)]."""
        r = self.r
        base, params, cons = self.base, self.params, self.constraints
        result = []
        nder = r.randint(3, 7)
        for i in range(nder):
            mode = r.choice(["pass", "pass", "fix", "permute", "wrap", "level2", "extra_param", "merge", "override", "override_bad", "classmerge", "mixin"])
            dname = f"D{i}"
            own = list(params)
            own_cons = dict(cons)
            args = list(params)
            if mode == "fix":
                k = r.randrange(len(params))
                args[k] = self.pick_args(cons[params[k]])
                own = [p for j, p in enumerate(params) if j != k] or []
            elif mode == "permute" and len(params) > 1:
                args = list(params)
                r.shuffle(args)
                if any(cons[a] != cons[p] for a, p in zip(args, params)):
                    args = list(params)
            elif mode == "wrap":
                k = r.randrange(len(params))
                if cons[params[k]] is None:
                    args[k] = r.choice([f"{params[k]}[]", f"Box<{params[k]}>", f"{params[k]} | undefined", f"Partial<{params[k]}>", f"[{params[k]}]"])
            elif mode == "extra_param":
                own = params + ["W"]
                own_cons["W"] = None
            own_decl = ", ".join(f"{p} extends string" if own_cons.get(p) == "string" else p for p in own)
            own_decl_full = f"<{own_decl}>" if own else ""
            extra = []
            nextra = r.randint(0, 3)
            for _ in range(nextra):
                X = r.choice(own) if own else "number"
                extra.append(f"{self.name('dx')}: {X};")
            if self.has_tag and r.random() < 0.5:
                extra.append(f"tag: {r.choice(own) if own else 'string'};")
            if mode in ("override", "override_bad"):
                extra.append(self.override_member(mode == "override_bad"))
            bargs = ", ".join(args)
            if self.kind == "interface" or mode in ("classmerge",):
                if mode == "classmerge" and self.kind == "interface":
                    out.append(f"interface {dname}{own_decl_full} extends {base}<{bargs}> {{ {' '.join(extra)} }}")
                    out.append(f"declare class {dname}{own_decl_full} {{ {self.name('cm')}: number; }}")
                    self.features["d_classmerge"] += 1
                else:
                    out.append(f"interface {dname}{own_decl_full} extends {base}<{bargs}> {{ {' '.join(extra)} }}")
                    if mode == "merge":
                        X = own[0] if own else "number"
                        out.append(f"interface {dname}{own_decl_full} {{ {self.name('dm')}: {X}; }}")
            elif mode == "mixin" and "abstract" not in self.kind:
                out.append(f"declare function mix{i}<C extends new (...a: any[]) => object>(c: C): C & (new (...a: any[]) => {{ {self.name('mx')}: number }});")
                out.append(f"declare const {dname}Base: typeof {base};")
                out.append(f"declare class {dname}{own_decl_full} extends mix{i}({dname}Base)<{bargs}> {{ {' '.join(extra)} }}")
            else:
                ab = "abstract " if "abstract" in self.kind else ""
                out.append(f"declare {ab}class {dname}{own_decl_full} extends {base}<{bargs}> {{ {' '.join(extra)} }}")
            self.features["d_" + mode] += 1
            result.append((dname, own, own_cons, args))
            if mode == "level2":
                d2 = f"D{i}x"
                if self.kind == "interface":
                    out.append(f"interface {d2}{own_decl_full} extends {dname}{('<' + ', '.join(own) + '>') if own else ''} {{ {self.name('l2')}: 1; }}")
                else:
                    ab = "abstract " if "abstract" in self.kind else ""
                    out.append(f"declare {ab}class {d2}{own_decl_full} extends {dname}{('<' + ', '.join(own) + '>') if own else ''} {{ {self.name('l2')}: 1; }}")
                result.append((d2, own, own_cons, args))
        return result

    def override_member(self, bad):
        r = self.r
        # Override a padding-like member name that may exist: redeclare a fresh `tag`-free member, or narrow `pad`.
        n = self.name("ov")
        return f"{n}: {'string' if bad else 'number'};"

    def sites(self, out, derived):
        r = self.r
        base, params, cons = self.base, self.params, self.constraints
        k = 0
        nsites = r.randint(60, 120)
        for _ in range(nsites):
            dname, own, own_cons, args = r.choice(derived)
            # Choose source arguments for D's own parameters and target arguments for the base.
            src = {}
            tgt = []
            for p in own:
                src[p] = None
            # Pair per base parameter where the base argument is one of D's own parameters passed through.
            for j, p in enumerate(params):
                a, b = self.pair(cons[p])
                tgt.append(b)
                arg = args[j]
                if arg in src and src[arg] is None:
                    src[arg] = a
            for p in own:
                if src[p] is None:
                    src[p] = self.pick_args(own_cons.get(p))
            sargs = ", ".join(src[p] for p in own)
            stype = f"{dname}<{sargs}>" if own else dname
            ttype = f"{base}<{', '.join(tgt)}>"
            k += 1
            form = r.random()
            if form < 0.55:
                out.append(f"declare const s{k}: {stype};\nexport const t{k}: {ttype} = s{k};")
            elif form < 0.65:
                out.append(f"export type C{k} = {stype} extends {ttype} ? \"yes\" : \"no\";\nexport const c{k}: C{k} = null!;")
                self.features["site_conditional"] += 1
            elif form < 0.72:
                out.append(f"declare const s{k}: {stype};\ndeclare const u{k}: {ttype};\nexport const a{k} = [s{k}, u{k}];\nexport const p{k} = pick(s{k}, u{k});")
                self.features["site_subtype"] += 1
            elif form < 0.78:
                out.append(f"declare const s{k}: {stype};\nexport const k{k} = s{k} as {ttype};")
                self.features["site_cast"] += 1
            elif form < 0.86:
                out.append(f"declare const s{k}: {{ x: {stype} }};\nexport const t{k}: {{ x: {ttype} }} = s{k};")
                self.features["site_nested"] += 1
            else:
                # Generic function: type parameters as arguments.
                if not own:
                    out.append(f"declare const s{k}: {stype};\nexport const t{k}: {ttype} = s{k};")
                    continue
                gp = []
                sa = []
                ta = []
                for j, p in enumerate(params):
                    g = f"G{j}"
                    h = f"H{j}"
                    c = " extends string" if cons[p] == "string" else ""
                    gp.append(f"{g}{c}, {h} extends {g}")
                for p in own:
                    j = params.index(p) if p in params else 0
                    sa.append(r.choice([f"H{j}", f"G{j}", f"G{j}"]))
                for j in range(len(params)):
                    ta.append(r.choice([f"G{j}", f"G{j}", f"H{j}", "any", "unknown"]) if cons[params[j]] is None else r.choice([f"G{j}", f"H{j}"]))
                own_args = ", ".join(sa)
                out.append(f"export function g{k}<{', '.join(gp)}>(s: {dname}<{own_args}>): {base}<{', '.join(ta)}> {{ return s; }}")
                self.features["site_generic"] += 1


def generate(seed, exclude=()):
    g = Gen(seed, exclude)
    src, cfg = g.program()
    return src, cfg, g


EXTRA_ENV = {}


def run_tsrs(tsrs, d, mode, log=None, min_members=None):
    env = dict(os.environ, TSRS_DERIVED_VARIANCE=mode, **EXTRA_ENV)
    env.pop("TSRS_CHECKER_ASSIGNMENT", None)
    if log:
        env["TSRS_DERIVED_VARIANCE_LOG"] = log
    if min_members is not None:
        env["TSRS_DERIVED_VARIANCE_MIN_MEMBERS"] = str(min_members)
    try:
        p = subprocess.run([tsrs, "-p", ".", "--pretty", "false", "--singleThreaded"], cwd=d, env=env, capture_output=True, timeout=TIMEOUT)
        return p.returncode, p.stdout.decode("utf-8", "replace"), p.stderr.decode("utf-8", "replace")
    except subprocess.TimeoutExpired:
        return -1, "", "timeout"


def one(args):
    seed, tsrs, exclude, onoff, min_members, extra_env = args
    EXTRA_ENV.update(extra_env)
    src, cfg, g = generate(seed, exclude)
    d = tempfile.mkdtemp(prefix=f"dv{seed}-", dir="/tmp/opencode" if os.path.isdir("/tmp/opencode") else None)
    try:
        with open(os.path.join(d, "a.ts"), "w") as f:
            f.write(src)
        with open(os.path.join(d, "tsconfig.json"), "w") as f:
            f.write(cfg)
        log = os.path.join(d, "dv.log")
        code, out, err = run_tsrs(tsrs, d, "shadow", log, min_members)
        decisions = 0
        if os.path.exists(log):
            with open(log) as f:
                decisions = sum(1 for line in f if line.startswith("D "))
        disagreements = [line for line in err.splitlines() if "would relate" in line]
        res = {"seed": seed, "decisions": decisions, "disagreements": disagreements, "features": dict(g.features),
               "opts": g.opts_name, "ts2636": "TS2636" in out, "crash": code not in (0, 1, 2, 7), "onoff": None, "code": code}
        if code == -1 or res["crash"]:
            res["err"] = err[-2000:]
        if onoff:
            c1, o1, _ = run_tsrs(tsrs, d, "off", None, min_members)
            c2, o2, _ = run_tsrs(tsrs, d, "on", None, min_members)
            res["onoff"] = (o1 == o2 and c1 == c2)
            res["shadow_eq_off"] = (o1 == out)
            if not res["onoff"]:
                res["onoff_diff"] = [o1[-3000:], o2[-3000:]]
        return res
    finally:
        shutil.rmtree(d, ignore_errors=True)


MEMBER_RE = re.compile(r"(?:\| |; )([a-z][a-z_]*?)_\d+: ")


def culprit_features(line):
    # "... | members: name_n: S vs T ; name_m: ..." -> {feature}; member names are `<feature>_<n>`.
    feats = set(MEMBER_RE.findall(line))
    return feats or {"(none)"}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["gen", "run"])
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--start", type=int, default=0)
    ap.add_argument("--count", type=int, default=100)
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 2)
    ap.add_argument("--out", default="/tmp/dvfuzz")
    ap.add_argument("--tsrs", default=TSRS)
    ap.add_argument("--exclude", default="")
    ap.add_argument("--min-members", type=int, default=None)
    ap.add_argument("--onoff-every", type=int, default=1)
    ap.add_argument("--summary", default=None, help="append the JSON summary to this file")
    ap.add_argument("--env", action="append", default=[], help="KEY=VALUE for tsrs (e.g. TSRS_DERIVED_VARIANCE_NO_GUARD=4,5)")
    a = ap.parse_args()
    exclude = [x for x in a.exclude.split(",") if x]
    if a.cmd == "gen":
        src, cfg, _ = generate(a.seed, exclude)
        os.makedirs(a.out, exist_ok=True)
        with open(os.path.join(a.out, "a.ts"), "w") as f:
            f.write(src)
        with open(os.path.join(a.out, "tsconfig.json"), "w") as f:
            f.write(cfg)
        print(a.out)
        return 0
    os.makedirs(a.out, exist_ok=True)
    tot = collections.Counter()
    per_feature = collections.Counter()
    culprits = collections.Counter()
    per_opts = collections.Counter()
    fired = 0
    findings = 0
    extra_env = dict(e.split("=", 1) for e in a.env)
    jobs = [(s, a.tsrs, exclude, (s - a.start) % a.onoff_every == 0, a.min_members, extra_env) for s in range(a.start, a.start + a.count)]
    with concurrent.futures.ProcessPoolExecutor(a.jobs) as ex:
        for res in ex.map(one, jobs, chunksize=4):
            tot["programs"] += 1
            tot["decisions"] += res["decisions"]
            per_opts[res["opts"]] += res["decisions"]
            if res["decisions"]:
                fired += 1
                for f in res["features"]:
                    per_feature[f] += res["decisions"]
            bad = bool(res["disagreements"]) or res["crash"] or res["onoff"] is False or res.get("shadow_eq_off") is False
            if res["onoff"] is not None:
                tot["onoff_compared"] += 1
                tot["onoff_differ"] += res["onoff"] is False
                # An on/off difference that shadow mode did not report would be a kind of difference it cannot see.
                tot["onoff_differ_unexplained"] += res["onoff"] is False and not res["disagreements"]
                tot["shadow_ne_off"] += res.get("shadow_eq_off") is False
            tot["disagreements"] += len(res["disagreements"])
            for line in res["disagreements"]:
                # TS2636: a variance annotation the declaration does not satisfy; TypeScript reports it and then
                # trusts the annotation, so every comparison by variances is suspect in that program.
                key = ",".join(sorted(culprit_features(line)))
                culprits[("[TS2636] " if res["ts2636"] else "") + key] += 1
            if res["crash"]:
                tot["crashes"] += 1
            if bad:
                findings += 1
                fd = os.path.join(a.out, str(res["seed"]))
                os.makedirs(fd, exist_ok=True)
                src, cfg, _ = generate(res["seed"], exclude)
                with open(os.path.join(fd, "a.ts"), "w") as f:
                    f.write(src)
                with open(os.path.join(fd, "tsconfig.json"), "w") as f:
                    f.write(cfg)
                with open(os.path.join(fd, "report.json"), "w") as f:
                    json.dump(res, f, indent=1)
    tot["programs_where_it_fired"] = fired
    tot["findings"] = findings
    summary = {"start": a.start, "count": a.count, "exclude": exclude, "env": extra_env, "min_members": a.min_members, "totals": dict(tot), "culprits": dict(culprits.most_common()),
               "decisions_per_feature": dict(per_feature.most_common()), "decisions_per_options": dict(per_opts)}
    print(json.dumps(summary, indent=1))
    if a.summary:
        with open(a.summary, "a") as f:
            f.write(json.dumps(summary) + "\n")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
