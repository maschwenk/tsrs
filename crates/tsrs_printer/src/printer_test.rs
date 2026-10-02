// Port of the cheap cases of printer_test.go: the TestEmit table (parse -> print round trips) and the
// synthesized-node tests that only need the plain AST factory or the EmitContext factory.

use tsrs_ast::*;
use tsrs_core::tspath::Path;
use tsrs_core::*;

use crate::*;
use tsrs_ast::NodeFactory;

fn parse_type_script(text: &str, jsx: bool) -> P<SourceFile> {
    let file_name = if jsx { "/main.tsx" } else { "/main.ts" };
    tsrs_parser::parse_source_file(
        SourceFileParseOptions { file_name: file_name.to_string(), path: Path::new(file_name.to_string()), external_module_indicator_options: Default::default() },
        text,
        get_script_kind_from_file_name(file_name),
    )
}

// Go emittestutil.CheckEmit: prints with LF newlines, trims the trailing newline, and checks that the output reparses
// without diagnostics.
fn check_emit(emit_context: Option<P<EmitContext>>, file: P<SourceFile>, expected: &str) -> Result<(), String> {
    let mut printer = new_printer(PrinterOptions { new_line: NewLineKind::LF, ..Default::default() }, PrintHandlers::default(), emit_context);
    let text = printer.emit_source_file(file);
    let actual = text.strip_suffix('\n').unwrap_or(&text);
    if actual != expected {
        return Err(format!("expected {:?}, got {:?}", expected, actual));
    }
    let file2 = parse_type_script(&text, file.language_variant() == LanguageVariant::JSX);
    if !file2.diagnostics().is_empty() {
        return Err(format!("error on reparse of {:?}", text));
    }
    Ok(())
}

fn synthetic_source_file(f: &NodeFactory, statements: Vec<P<Node>>) -> P<SourceFile> {
    let statements = f.new_node_list(statements);
    let eof = f.new_token(Kind::EndOfFile);
    f.new_source_file(SourceFileParseOptions { file_name: "/file.ts".to_string(), path: Path::new("/file.ts".to_string()), external_module_indicator_options: Default::default() }, "", statements, eof)
        .as_source_file_p()
}

#[rustfmt::skip]
const EMIT_CASES: &[(&str, &str, &str, bool)] = &[
    ("StringLiteral#1", ";\"test\"", ";\n\"test\";", false),
    ("StringLiteral#2", ";'test'", ";\n'test';", false),
    ("NumericLiteral#1", "0", "0;", false),
    ("NumericLiteral#2", "10_000", "10000;", false),
    ("BigIntLiteral#1", "0n", "0n;", false),
    ("BigIntLiteral#2", "10_000n", "10000n;", false),
    ("BooleanLiteral#1", "true", "true;", false),
    ("BooleanLiteral#2", "false", "false;", false),
    ("NoSubstitutionTemplateLiteral", "``", "``;", false),
    ("NoSubstitutionTemplateLiteral#2", "`\n`", "`\n`;", false),
    ("RegularExpressionLiteral#1", "/a/", "/a/;", false),
    ("RegularExpressionLiteral#2", "/a/g", "/a/g;", false),
    ("NullLiteral", "null", "null;", false),
    ("ThisExpression", "this", "this;", false),
    ("SuperExpression", "super()", "super();", false),
    ("ImportExpression", "import()", "import();", false),
    ("PropertyAccess#1", "a.b", "a.b;", false),
    ("PropertyAccess#2", "a.#b", "a.#b;", false),
    ("PropertyAccess#3", "a?.b", "a?.b;", false),
    ("PropertyAccess#4", "a?.b.c", "a?.b.c;", false),
    ("PropertyAccess#5", "1..b", "1..b;", false),
    ("PropertyAccess#6", "1.0.b", "1.0.b;", false),
    ("PropertyAccess#7", "0x1.b", "0x1.b;", false),
    ("PropertyAccess#8", "0b1.b", "0b1.b;", false),
    ("PropertyAccess#9", "0o1.b", "0o1.b;", false),
    ("PropertyAccess#10", "10e1.b", "10e1.b;", false),
    ("PropertyAccess#11", "10E1.b", "10E1.b;", false),
    ("PropertyAccess#12", "a.b?.c", "a.b?.c;", false),
    ("PropertyAccess#13", "a\n.b", "a\n    .b;", false),
    ("PropertyAccess#14", "a.\nb", "a.\n    b;", false),
    ("ElementAccess#1", "a[b]", "a[b];", false),
    ("ElementAccess#2", "a?.[b]", "a?.[b];", false),
    ("ElementAccess#3", "a?.[b].c", "a?.[b].c;", false),
    ("CallExpression#1", "a()", "a();", false),
    ("CallExpression#2", "a<T>()", "a<T>();", false),
    ("CallExpression#3", "a(b)", "a(b);", false),
    ("CallExpression#4", "a<T>(b)", "a<T>(b);", false),
    ("CallExpression#5", "a(b).c", "a(b).c;", false),
    ("CallExpression#6", "a<T>(b).c", "a<T>(b).c;", false),
    ("CallExpression#7", "a?.(b)", "a?.(b);", false),
    ("CallExpression#8", "a?.<T>(b)", "a?.<T>(b);", false),
    ("CallExpression#9", "a?.(b).c", "a?.(b).c;", false),
    ("CallExpression#10", "a?.<T>(b).c", "a?.<T>(b).c;", false),
    ("CallExpression#11", "a<T, U>()", "a<T, U>();", false),
    ("CallExpression#13", "a?.b()", "a?.b();", false),
    ("NewExpression#1", "new a", "new a;", false),
    ("NewExpression#2", "new a.b", "new a.b;", false),
    ("NewExpression#3", "new a()", "new a();", false),
    ("NewExpression#4", "new a.b()", "new a.b();", false),
    ("NewExpression#5", "new a<T>()", "new a<T>();", false),
    ("NewExpression#6", "new a.b<T>()", "new a.b<T>();", false),
    ("NewExpression#7", "new a(b)", "new a(b);", false),
    ("NewExpression#8", "new a.b(c)", "new a.b(c);", false),
    ("NewExpression#9", "new a<T>(b)", "new a<T>(b);", false),
    ("NewExpression#10", "new a.b<T>(c)", "new a.b<T>(c);", false),
    ("NewExpression#11", "new a(b).c", "new a(b).c;", false),
    ("NewExpression#12", "new a<T>(b).c", "new a<T>(b).c;", false),
    ("TaggedTemplateExpression#1", "tag``", "tag ``;", false),
    ("TaggedTemplateExpression#2", "tag<T>``", "tag<T> ``;", false),
    ("TypeAssertionExpression#1", "<T>a", "<T>a;", false),
    ("FunctionExpression#1", "(function(){})", "(function () { });", false),
    ("FunctionExpression#2", "(function f(){})", "(function f() { });", false),
    ("FunctionExpression#3", "(function*f(){})", "(function* f() { });", false),
    ("FunctionExpression#4", "(async function f(){})", "(async function f() { });", false),
    ("FunctionExpression#5", "(async function*f(){})", "(async function* f() { });", false),
    ("FunctionExpression#6", "(function<T>(){})", "(function <T>() { });", false),
    ("FunctionExpression#7", "(function(a){})", "(function (a) { });", false),
    ("FunctionExpression#8", "(function():T{})", "(function (): T { });", false),
    ("ArrowFunction#1", "a=>{}", "a => { };", false),
    ("ArrowFunction#2", "()=>{}", "() => { };", false),
    ("ArrowFunction#3", "(a)=>{}", "(a) => { };", false),
    ("ArrowFunction#4", "<T>(a)=>{}", "<T>(a) => { };", false),
    ("ArrowFunction#5", "async a=>{}", "async (a) => { };", false),
    ("ArrowFunction#6", "async()=>{}", "async () => { };", false),
    ("ArrowFunction#7", "async<T>()=>{}", "async <T>() => { };", false),
    ("ArrowFunction#8", "():T=>{}", "(): T => { };", false),
    ("ArrowFunction#9", "()=>a", "() => a;", false),
    ("DeleteExpression", "delete a", "delete a;", false),
    ("TypeOfExpression", "typeof a", "typeof a;", false),
    ("VoidExpression", "void a", "void a;", false),
    ("AwaitExpression", "await a", "await a;", false),
    ("PrefixUnaryExpression#1", "+a", "+a;", false),
    ("PrefixUnaryExpression#2", "++a", "++a;", false),
    ("PrefixUnaryExpression#3", "+ +a", "+ +a;", false),
    ("PrefixUnaryExpression#4", "+ ++a", "+ ++a;", false),
    ("PrefixUnaryExpression#5", "-a", "-a;", false),
    ("PrefixUnaryExpression#6", "--a", "--a;", false),
    ("PrefixUnaryExpression#7", "- -a", "- -a;", false),
    ("PrefixUnaryExpression#8", "- --a", "- --a;", false),
    ("PrefixUnaryExpression#9", "+-a", "+-a;", false),
    ("PrefixUnaryExpression#10", "+--a", "+--a;", false),
    ("PrefixUnaryExpression#11", "-+a", "-+a;", false),
    ("PrefixUnaryExpression#12", "-++a", "-++a;", false),
    ("PrefixUnaryExpression#13", "~a", "~a;", false),
    ("PrefixUnaryExpression#14", "!a", "!a;", false),
    ("PostfixUnaryExpression#1", "a++", "a++;", false),
    ("PostfixUnaryExpression#2", "a--", "a--;", false),
    ("BinaryExpression#1", "a,b", "a, b;", false),
    ("BinaryExpression#2", "a+b", "a + b;", false),
    ("BinaryExpression#3", "a**b", "a ** b;", false),
    ("BinaryExpression#4", "a instanceof b", "a instanceof b;", false),
    ("BinaryExpression#5", "a in b", "a in b;", false),
    ("BinaryExpression#6", "a\n&& b", "a\n    && b;", false),
    ("BinaryExpression#7", "a &&\nb", "a &&\n    b;", false),
    ("ConditionalExpression#1", "a?b:c", "a ? b : c;", false),
    ("ConditionalExpression#2", "a\n?b:c", "a\n    ? b : c;", false),
    ("ConditionalExpression#3", "a?\nb:c", "a ?\n    b : c;", false),
    ("ConditionalExpression#4", "a?b\n:c", "a ? b\n    : c;", false),
    ("ConditionalExpression#5", "a?b:\nc", "a ? b :\n    c;", false),
    ("TemplateExpression#1", "`a${b}c`", "`a${b}c`;", false),
    ("TemplateExpression#2", "`a${b}c${d}e`", "`a${b}c${d}e`;", false),
    ("YieldExpression#1", "(function*() { yield })", "(function* () { yield; });", false),
    ("YieldExpression#2", "(function*() { yield a })", "(function* () { yield a; });", false),
    ("YieldExpression#3", "(function*() { yield*a })", "(function* () { yield* a; });", false),
    ("SpreadElement", "[...a]", "[...a];", false),
    ("ClassExpression#1", "(class {})", "(class {\n});", false),
    ("ClassExpression#2", "(class a {})", "(class a {\n});", false),
    ("ClassExpression#3", "(class<T>{})", "(class<T> {\n});", false),
    ("ClassExpression#4", "(class a<T>{})", "(class a<T> {\n});", false),
    ("ClassExpression#5", "(class extends b {})", "(class extends b {\n});", false),
    ("ClassExpression#6", "(class a extends b {})", "(class a extends b {\n});", false),
    ("ClassExpression#7", "(class implements b {})", "(class implements b {\n});", false),
    ("ClassExpression#8", "(class a implements b {})", "(class a implements b {\n});", false),
    ("ClassExpression#9", "(class implements b, c {})", "(class implements b, c {\n});", false),
    ("ClassExpression#10", "(class a implements b, c {})", "(class a implements b, c {\n});", false),
    ("ClassExpression#11", "(class extends b implements c, d {})", "(class extends b implements c, d {\n});", false),
    ("ClassExpression#12", "(class a extends b implements c, d {})", "(class a extends b implements c, d {\n});", false),
    ("ClassExpression#13", "(@a class {})", "(\n@a\nclass {\n});", false),
    ("OmittedExpression", "[,]", "[,];", false),
    ("ExpressionWithTypeArguments", "a<T>", "a<T>;", false),
    ("AsExpression", "a as T", "a as T;", false),
    ("SatisfiesExpression", "a satisfies T", "a satisfies T;", false),
    ("NonNullExpression", "a!", "a!;", false),
    ("MetaProperty#1", "new.target", "new.target;", false),
    ("MetaProperty#2", "import.meta", "import.meta;", false),
    ("ArrayLiteralExpression#1", "[]", "[];", false),
    ("ArrayLiteralExpression#2", "[a]", "[a];", false),
    ("ArrayLiteralExpression#3", "[a,]", "[a,];", false),
    ("ArrayLiteralExpression#4", "[,a]", "[, a];", false),
    ("ArrayLiteralExpression#5", "[...a]", "[...a];", false),
    ("ArrayLiteralExpression#6", "const array = [/* comment */];", "const array = [ /* comment */];", false),
    ("ObjectLiteralExpression#1", "({})", "({});", false),
    ("ObjectLiteralExpression#2", "({a,})", "({ a, });", false),
    ("ShorthandPropertyAssignment", "({a})", "({ a });", false),
    ("PropertyAssignment", "({a:b})", "({ a: b });", false),
    ("SpreadAssignment", "({...a})", "({ ...a });", false),
    ("Block", "{}", "{ }", false),
    ("VariableStatement#1", "var a", "var a;", false),
    ("VariableStatement#2", "let a", "let a;", false),
    ("VariableStatement#3", "const a = b", "const a = b;", false),
    ("VariableStatement#4", "using a = b", "using a = b;", false),
    ("VariableStatement#5", "await using a = b", "await using a = b;", false),
    ("EmptyStatement", ";", ";", false),
    ("IfStatement#1", "if(a);", "if (a)\n    ;", false),
    ("IfStatement#2", "if(a);else;", "if (a)\n    ;\nelse\n    ;", false),
    ("IfStatement#3", "if(a);else{}", "if (a)\n    ;\nelse { }", false),
    ("IfStatement#4", "if(a);else if(b);", "if (a)\n    ;\nelse if (b)\n    ;", false),
    ("IfStatement#5", "if(a);else if(b) {}", "if (a)\n    ;\nelse if (b) { }", false),
    ("IfStatement#6", "if(a) {}", "if (a) { }", false),
    ("IfStatement#7", "if(a) {} else;", "if (a) { }\nelse\n    ;", false),
    ("IfStatement#8", "if(a) {} else {}", "if (a) { }\nelse { }", false),
    ("IfStatement#9", "if(a) {} else if(b);", "if (a) { }\nelse if (b)\n    ;", false),
    ("IfStatement#10", "if(a) {} else if(b){}", "if (a) { }\nelse if (b) { }", false),
    ("DoStatement#1", "do;while(a);", "do\n    ;\nwhile (a);", false),
    ("DoStatement#2", "do {} while(a);", "do { } while (a);", false),
    ("WhileStatement#1", "while(a);", "while (a)\n    ;", false),
    ("WhileStatement#2", "while(a) {}", "while (a) { }", false),
    ("ForStatement#1", "for(;;);", "for (;;)\n    ;", false),
    ("ForStatement#2", "for(a;;);", "for (a;;)\n    ;", false),
    ("ForStatement#3", "for(var a;;);", "for (var a;;)\n    ;", false),
    ("ForStatement#4", "for(;a;);", "for (; a;)\n    ;", false),
    ("ForStatement#5", "for(;;a);", "for (;; a)\n    ;", false),
    ("ForStatement#6", "for(;;){}", "for (;;) { }", false),
    ("ForInStatement#1", "for(a in b);", "for (a in b)\n    ;", false),
    ("ForInStatement#2", "for(var a in b);", "for (var a in b)\n    ;", false),
    ("ForInStatement#3", "for(a in b){}", "for (a in b) { }", false),
    ("ForOfStatement#1", "for(a of b);", "for (a of b)\n    ;", false),
    ("ForOfStatement#2", "for(var a of b);", "for (var a of b)\n    ;", false),
    ("ForOfStatement#3", "for(a of b){}", "for (a of b) { }", false),
    ("ForOfStatement#4", "for await(a of b);", "for await (a of b)\n    ;", false),
    ("ForOfStatement#5", "for await(var a of b);", "for await (var a of b)\n    ;", false),
    ("ForOfStatement#6", "for await(a of b){}", "for await (a of b) { }", false),
    ("ContinueStatement#1", "continue", "continue;", false),
    ("ContinueStatement#2", "continue a", "continue a;", false),
    ("BreakStatement#1", "break", "break;", false),
    ("BreakStatement#2", "break a", "break a;", false),
    ("ReturnStatement#1", "return", "return;", false),
    ("ReturnStatement#2", "return a", "return a;", false),
    ("WithStatement#1", "with(a);", "with (a)\n    ;", false),
    ("WithStatement#2", "with(a){}", "with (a) { }", false),
    ("SwitchStatement", "switch (a) {}", "switch (a) {\n}", false),
    ("CaseClause#1", "switch (a) {case b:}", "switch (a) {\n    case b:\n}", false),
    ("CaseClause#2", "switch (a) {case b:;}", "switch (a) {\n    case b: ;\n}", false),
    ("DefaultClause#1", "switch (a) {default:}", "switch (a) {\n    default:\n}", false),
    ("DefaultClause#2", "switch (a) {default:;}", "switch (a) {\n    default: ;\n}", false),
    ("LabeledStatement", "a:;", "a: ;", false),
    ("ThrowStatement", "throw a", "throw a;", false),
    ("TryStatement#1", "try {} catch {}", "try { }\ncatch { }", false),
    ("TryStatement#2", "try {} finally {}", "try { }\nfinally { }", false),
    ("TryStatement#3", "try {} catch {} finally {}", "try { }\ncatch { }\nfinally { }", false),
    ("DebuggerStatement", "debugger", "debugger;", false),
    ("FunctionDeclaration#1", "export default function(){}", "export default function () { }", false),
    ("FunctionDeclaration#2", "function f(){}", "function f() { }", false),
    ("FunctionDeclaration#3", "function*f(){}", "function* f() { }", false),
    ("FunctionDeclaration#4", "async function f(){}", "async function f() { }", false),
    ("FunctionDeclaration#5", "async function*f(){}", "async function* f() { }", false),
    ("FunctionDeclaration#6", "function f<T>(){}", "function f<T>() { }", false),
    ("FunctionDeclaration#7", "function f(a){}", "function f(a) { }", false),
    ("FunctionDeclaration#8", "function f():T{}", "function f(): T { }", false),
    ("FunctionDeclaration#9", "function f();", "function f();", false),
    ("ClassDeclaration#1", "class a {}", "class a {\n}", false),
    ("ClassDeclaration#2", "class a<T>{}", "class a<T> {\n}", false),
    ("ClassDeclaration#3", "class a extends b {}", "class a extends b {\n}", false),
    ("ClassDeclaration#4", "class a implements b {}", "class a implements b {\n}", false),
    ("ClassDeclaration#5", "class a implements b, c {}", "class a implements b, c {\n}", false),
    ("ClassDeclaration#6", "class a extends b implements c, d {}", "class a extends b implements c, d {\n}", false),
    ("ClassDeclaration#7", "export default class {}", "export default class {\n}", false),
    ("ClassDeclaration#8", "export default class<T>{}", "export default class<T> {\n}", false),
    ("ClassDeclaration#9", "export default class extends b {}", "export default class extends b {\n}", false),
    ("ClassDeclaration#10", "export default class implements b {}", "export default class implements b {\n}", false),
    ("ClassDeclaration#11", "export default class implements b, c {}", "export default class implements b, c {\n}", false),
    ("ClassDeclaration#12", "export default class extends b implements c, d {}", "export default class extends b implements c, d {\n}", false),
    ("ClassDeclaration#13", "@a class b {}", "@a\nclass b {\n}", false),
    ("ClassDeclaration#14", "@a export class b {}", "@a\nexport class b {\n}", false),
    ("ClassDeclaration#15", "export @a class b {}", "export \n@a\nclass b {\n}", false),
    ("InterfaceDeclaration#1", "interface a {}", "interface a {\n}", false),
    ("InterfaceDeclaration#2", "interface a<T>{}", "interface a<T> {\n}", false),
    ("InterfaceDeclaration#3", "interface a extends b {}", "interface a extends b {\n}", false),
    ("InterfaceDeclaration#4", "interface a extends b, c {}", "interface a extends b, c {\n}", false),
    ("TypeAliasDeclaration#1", "type a = b", "type a = b;", false),
    ("TypeAliasDeclaration#2", "type a<T> = b", "type a<T> = b;", false),
    ("EnumDeclaration#1", "enum a{}", "enum a {\n}", false),
    ("EnumDeclaration#2", "enum a{b}", "enum a {\n    b\n}", false),
    ("EnumDeclaration#3", "enum a{b=c}", "enum a {\n    b = c\n}", false),
    ("ModuleDeclaration#1", "module a{}", "module a { }", false),
    ("ModuleDeclaration#2", "module a.b{}", "module a.b { }", false),
    ("ModuleDeclaration#3", "module \"a\";", "module \"a\";", false),
    ("ModuleDeclaration#4", "module \"a\"{}", "module \"a\" { }", false),
    ("ModuleDeclaration#5", "namespace a{}", "namespace a { }", false),
    ("ModuleDeclaration#6", "namespace a.b{}", "namespace a.b { }", false),
    ("ModuleDeclaration#7", "global;", "global;", false),
    ("ModuleDeclaration#8", "global{}", "global { }", false),
    ("ImportEqualsDeclaration#1", "import a = b", "import a = b;", false),
    ("ImportEqualsDeclaration#2", "import a = b.c", "import a = b.c;", false),
    ("ImportEqualsDeclaration#3", "import a = require(\"b\")", "import a = require(\"b\");", false),
    ("ImportEqualsDeclaration#4", "export import a = b", "export import a = b;", false),
    ("ImportEqualsDeclaration#5", "export import a = require(\"b\")", "export import a = require(\"b\");", false),
    ("ImportEqualsDeclaration#6", "import type a = b", "import type a = b;", false),
    ("ImportEqualsDeclaration#7", "import type a = b.c", "import type a = b.c;", false),
    ("ImportEqualsDeclaration#8", "import type a = require(\"b\")", "import type a = require(\"b\");", false),
    ("ImportDeclaration#1", "import \"a\"", "import \"a\";", false),
    ("ImportDeclaration#2", "import a from \"b\"", "import a from \"b\";", false),
    ("ImportDeclaration#3", "import type a from \"b\"", "import type a from \"b\";", false),
    ("ImportDeclaration#4", "import * as a from \"b\"", "import * as a from \"b\";", false),
    ("ImportDeclaration#5", "import type * as a from \"b\"", "import type * as a from \"b\";", false),
    ("ImportDeclaration#6", "import {} from \"b\"", "import {} from \"b\";", false),
    ("ImportDeclaration#7", "import type {} from \"b\"", "import type {} from \"b\";", false),
    ("ImportDeclaration#8", "import { a } from \"b\"", "import { a } from \"b\";", false),
    ("ImportDeclaration#9", "import type { a } from \"b\"", "import type { a } from \"b\";", false),
    ("ImportDeclaration#8", "import { a as b } from \"c\"", "import { a as b } from \"c\";", false),
    ("ImportDeclaration#9", "import type { a as b } from \"c\"", "import type { a as b } from \"c\";", false),
    ("ImportDeclaration#10", "import { \"a\" as b } from \"c\"", "import { \"a\" as b } from \"c\";", false),
    ("ImportDeclaration#11", "import type { \"a\" as b } from \"c\"", "import type { \"a\" as b } from \"c\";", false),
    ("ImportDeclaration#12", "import a, {} from \"b\"", "import a, {} from \"b\";", false),
    ("ImportDeclaration#13", "import a, * as b from \"c\"", "import a, * as b from \"c\";", false),
    ("ImportDeclaration#14", "import {} from \"a\" with {}", "import {} from \"a\" with {};", false),
    ("ImportDeclaration#15", "import {} from \"a\" with { b: \"c\" }", "import {} from \"a\" with { b: \"c\" };", false),
    ("ImportDeclaration#16", "import {} from \"a\" with { \"b\": \"c\" }", "import {} from \"a\" with { \"b\": \"c\" };", false),
    ("ExportAssignment#1", "export = a", "export = a;", false),
    ("ExportAssignment#2", "export default a", "export default a;", false),
    ("NamespaceExportDeclaration", "export as namespace a", "export as namespace a;", false),
    ("ExportDeclaration#1", "export * from \"a\"", "export * from \"a\";", false),
    ("ExportDeclaration#2", "export type * from \"a\"", "export type * from \"a\";", false),
    ("ExportDeclaration#3", "export * as a from \"b\"", "export * as a from \"b\";", false),
    ("ExportDeclaration#4", "export type * as a from \"b\"", "export type * as a from \"b\";", false),
    ("ExportDeclaration#5", "export { } from \"a\"", "export {} from \"a\";", false),
    ("ExportDeclaration#6", "export type { } from \"a\"", "export type {} from \"a\";", false),
    ("ExportDeclaration#7", "export { a } from \"b\"", "export { a } from \"b\";", false),
    ("ExportDeclaration#8", "export { type a } from \"b\"", "export { type a } from \"b\";", false),
    ("ExportDeclaration#9", "export type { a } from \"b\"", "export type { a } from \"b\";", false),
    ("ExportDeclaration#10", "export { a as b } from \"c\"", "export { a as b } from \"c\";", false),
    ("ExportDeclaration#11", "export { type a as b } from \"c\"", "export { type a as b } from \"c\";", false),
    ("ExportDeclaration#12", "export type { a as b } from \"c\"", "export type { a as b } from \"c\";", false),
    ("ExportDeclaration#13", "export { a as \"b\" } from \"c\"", "export { a as \"b\" } from \"c\";", false),
    ("ExportDeclaration#14", "export { type a as \"b\" } from \"c\"", "export { type a as \"b\" } from \"c\";", false),
    ("ExportDeclaration#15", "export type { a as \"b\" } from \"c\"", "export type { a as \"b\" } from \"c\";", false),
    ("ExportDeclaration#16", "export { \"a\" } from \"b\"", "export { \"a\" } from \"b\";", false),
    ("ExportDeclaration#17", "export { type \"a\" } from \"b\"", "export { type \"a\" } from \"b\";", false),
    ("ExportDeclaration#18", "export type { \"a\" } from \"b\"", "export type { \"a\" } from \"b\";", false),
    ("ExportDeclaration#19", "export { \"a\" as b } from \"c\"", "export { \"a\" as b } from \"c\";", false),
    ("ExportDeclaration#20", "export { type \"a\" as b } from \"c\"", "export { type \"a\" as b } from \"c\";", false),
    ("ExportDeclaration#21", "export type { \"a\" as b } from \"c\"", "export type { \"a\" as b } from \"c\";", false),
    ("ExportDeclaration#22", "export { \"a\" as \"b\" } from \"c\"", "export { \"a\" as \"b\" } from \"c\";", false),
    ("ExportDeclaration#23", "export { type \"a\" as \"b\" } from \"c\"", "export { type \"a\" as \"b\" } from \"c\";", false),
    ("ExportDeclaration#24", "export type { \"a\" as \"b\" } from \"c\"", "export type { \"a\" as \"b\" } from \"c\";", false),
    ("ExportDeclaration#25", "export { }", "export {};", false),
    ("ExportDeclaration#26", "export type { }", "export type {};", false),
    ("ExportDeclaration#27", "export { a }", "export { a };", false),
    ("ExportDeclaration#28", "export { type a }", "export { type a };", false),
    ("ExportDeclaration#29", "export type { a }", "export type { a };", false),
    ("ExportDeclaration#30", "export { a as b }", "export { a as b };", false),
    ("ExportDeclaration#31", "export { type a as b }", "export { type a as b };", false),
    ("ExportDeclaration#32", "export type { a as b }", "export type { a as b };", false),
    ("ExportDeclaration#33", "export { a as \"b\" }", "export { a as \"b\" };", false),
    ("ExportDeclaration#34", "export { type a as \"b\" }", "export { type a as \"b\" };", false),
    ("ExportDeclaration#35", "export type { a as \"b\" }", "export type { a as \"b\" };", false),
    ("ExportDeclaration#36", "export {} from \"a\" with {}", "export {} from \"a\" with {};", false),
    ("ExportDeclaration#37", "export {} from \"a\" with { b: \"c\" }", "export {} from \"a\" with { b: \"c\" };", false),
    ("ExportDeclaration#38", "export {} from \"a\" with { \"b\": \"c\" }", "export {} from \"a\" with { \"b\": \"c\" };", false),
    ("KeywordTypeNode#1", "type T = any", "type T = any;", false),
    ("KeywordTypeNode#2", "type T = unknown", "type T = unknown;", false),
    ("KeywordTypeNode#3", "type T = never", "type T = never;", false),
    ("KeywordTypeNode#4", "type T = void", "type T = void;", false),
    ("KeywordTypeNode#5", "type T = undefined", "type T = undefined;", false),
    ("KeywordTypeNode#6", "type T = null", "type T = null;", false),
    ("KeywordTypeNode#7", "type T = object", "type T = object;", false),
    ("KeywordTypeNode#8", "type T = string", "type T = string;", false),
    ("KeywordTypeNode#9", "type T = symbol", "type T = symbol;", false),
    ("KeywordTypeNode#10", "type T = number", "type T = number;", false),
    ("KeywordTypeNode#11", "type T = bigint", "type T = bigint;", false),
    ("KeywordTypeNode#12", "type T = boolean", "type T = boolean;", false),
    ("KeywordTypeNode#13", "type T = intrinsic", "type T = intrinsic;", false),
    ("TypePredicateNode#1", "function f(): asserts a", "function f(): asserts a;", false),
    ("TypePredicateNode#2", "function f(): asserts a is b", "function f(): asserts a is b;", false),
    ("TypePredicateNode#3", "function f(): asserts this", "function f(): asserts this;", false),
    ("TypePredicateNode#4", "function f(): asserts this is b", "function f(): asserts this is b;", false),
    ("TypeReferenceNode#1", "type T = a", "type T = a;", false),
    ("TypeReferenceNode#2", "type T = a.b", "type T = a.b;", false),
    ("TypeReferenceNode#3", "type T = a<U>", "type T = a<U>;", false),
    ("TypeReferenceNode#4", "type T = a.b<U>", "type T = a.b<U>;", false),
    ("FunctionTypeNode#1", "type T = () => a", "type T = () => a;", false),
    ("FunctionTypeNode#2", "type T = <T>() => a", "type T = <T>() => a;", false),
    ("FunctionTypeNode#3", "type T = (a) => b", "type T = (a) => b;", false),
    ("ConstructorTypeNode#1", "type T = new () => a", "type T = new () => a;", false),
    ("ConstructorTypeNode#2", "type T = new <T>() => a", "type T = new <T>() => a;", false),
    ("ConstructorTypeNode#3", "type T = new (a) => b", "type T = new (a) => b;", false),
    ("ConstructorTypeNode#4", "type T = abstract new () => a", "type T = abstract new () => a;", false),
    ("TypeQueryNode#1", "type T = typeof a", "type T = typeof a;", false),
    ("TypeQueryNode#2", "type T = typeof a.b", "type T = typeof a.b;", false),
    ("TypeQueryNode#3", "type T = typeof a<U>", "type T = typeof a<U>;", false),
    ("TypeLiteralNode#1", "type T = {}", "type T = {};", false),
    ("TypeLiteralNode#2", "type T = {a}", "type T = {\n    a;\n};", false),
    ("ArrayTypeNode", "type T = a[]", "type T = a[];", false),
    ("TupleTypeNode#1", "type T = []", "type T = [\n];", false),
    ("TupleTypeNode#2", "type T = [a]", "type T = [\n    a\n];", false),
    ("TupleTypeNode#3", "type T = [a,]", "type T = [\n    a\n];", false),
    ("RestTypeNode", "type T = [...a]", "type T = [\n    ...a\n];", false),
    ("OptionalTypeNode", "type T = [a?]", "type T = [\n    a?\n];", false),
    ("NamedTupleMember#1", "type T = [a: b]", "type T = [\n    a: b\n];", false),
    ("NamedTupleMember#2", "type T = [a?: b]", "type T = [\n    a?: b\n];", false),
    ("NamedTupleMember#3", "type T = [...a: b]", "type T = [\n    ...a: b\n];", false),
    ("UnionTypeNode#1", "type T = a | b", "type T = a | b;", false),
    ("UnionTypeNode#2", "type T = a | b | c", "type T = a | b | c;", false),
    ("UnionTypeNode#3", "type T = | a | b", "type T = a | b;", false),
    ("IntersectionTypeNode#1", "type T = a & b", "type T = a & b;", false),
    ("IntersectionTypeNode#2", "type T = a & b & c", "type T = a & b & c;", false),
    ("IntersectionTypeNode#3", "type T = & a & b", "type T = a & b;", false),
    ("ConditionalTypeNode", "type T = a extends b ? c : d", "type T = a extends b ? c : d;", false),
    ("InferTypeNode#1", "type T = a extends infer b ? c : d", "type T = a extends infer b ? c : d;", false),
    ("InferTypeNode#2", "type T = a extends infer b extends c ? d : e", "type T = a extends infer b extends c ? d : e;", false),
    ("ParenthesizedTypeNode", "type T = (U)", "type T = (U);", false),
    ("ThisTypeNode", "type T = this", "type T = this;", false),
    ("TypeOperatorNode#1", "type T = keyof U", "type T = keyof U;", false),
    ("TypeOperatorNode#2", "type T = readonly U[]", "type T = readonly U[];", false),
    ("TypeOperatorNode#3", "type T = unique symbol", "type T = unique symbol;", false),
    ("IndexedAccessTypeNode", "type T = a[b]", "type T = a[b];", false),
    ("MappedTypeNode#1", "type T = { [a in b]: c }", "type T = {\n    [a in b]: c;\n};", false),
    ("MappedTypeNode#2", "type T = { [a in b as c]: d }", "type T = {\n    [a in b as c]: d;\n};", false),
    ("MappedTypeNode#3", "type T = { readonly [a in b]: c }", "type T = {\n    readonly [a in b]: c;\n};", false),
    ("MappedTypeNode#4", "type T = { +readonly [a in b]: c }", "type T = {\n    +readonly [a in b]: c;\n};", false),
    ("MappedTypeNode#5", "type T = { -readonly [a in b]: c }", "type T = {\n    -readonly [a in b]: c;\n};", false),
    ("MappedTypeNode#6", "type T = { [a in b]?: c }", "type T = {\n    [a in b]?: c;\n};", false),
    ("MappedTypeNode#7", "type T = { [a in b]+?: c }", "type T = {\n    [a in b]+?: c;\n};", false),
    ("MappedTypeNode#8", "type T = { [a in b]-?: c }", "type T = {\n    [a in b]-?: c;\n};", false),
    ("MappedTypeNode#9", "type T = { [a in b]: c; d }", "type T = {\n    [a in b]: c;\n    d;\n};", false),
    ("LiteralTypeNode#1", "type T = null", "type T = null;", false),
    ("LiteralTypeNode#2", "type T = true", "type T = true;", false),
    ("LiteralTypeNode#3", "type T = false", "type T = false;", false),
    ("LiteralTypeNode#4", "type T = \"\"", "type T = \"\";", false),
    ("LiteralTypeNode#5", "type T = ''", "type T = '';", false),
    ("LiteralTypeNode#6", "type T = ``", "type T = ``;", false),
    ("LiteralTypeNode#7", "type T = 0", "type T = 0;", false),
    ("LiteralTypeNode#8", "type T = 0n", "type T = 0n;", false),
    ("LiteralTypeNode#9", "type T = -0", "type T = -0;", false),
    ("LiteralTypeNode#10", "type T = -0n", "type T = -0n;", false),
    ("TemplateTypeNode#1", "type T = `a${b}c`", "type T = `a${b}c`;", false),
    ("TemplateTypeNode#2", "type T = `a${b}c${d}e`", "type T = `a${b}c${d}e`;", false),
    ("ImportTypeNode#1", "type T = import(a)", "type T = import(a);", false),
    ("ImportTypeNode#2", "type T = import(a).b", "type T = import(a).b;", false),
    ("ImportTypeNode#3", "type T = import(a).b<U>", "type T = import(a).b<U>;", false),
    ("ImportTypeNode#4", "type T = typeof import(a)", "type T = typeof import(a);", false),
    ("ImportTypeNode#5", "type T = typeof import(a).b", "type T = typeof import(a).b;", false),
    ("ImportTypeNode#6", "type T = import(a, { with: { } })", "type T = import(a, { with: {} });", false),
    ("ImportTypeNode#6", "type T = import(a, { with: { b: \"c\" } })", "type T = import(a, { with: { b: \"c\" } });", false),
    ("ImportTypeNode#7", "type T = import(a, { with: { \"b\": \"c\" } })", "type T = import(a, { with: { \"b\": \"c\" } });", false),
    ("PropertySignature#1", "interface I {a}", "interface I {\n    a;\n}", false),
    ("PropertySignature#2", "interface I {readonly a}", "interface I {\n    readonly a;\n}", false),
    ("PropertySignature#3", "interface I {\"a\"}", "interface I {\n    \"a\";\n}", false),
    ("PropertySignature#4", "interface I {'a'}", "interface I {\n    'a';\n}", false),
    ("PropertySignature#5", "interface I {0}", "interface I {\n    0;\n}", false),
    ("PropertySignature#6", "interface I {0n}", "interface I {\n    0n;\n}", false),
    ("PropertySignature#7", "interface I {[a]}", "interface I {\n    [a];\n}", false),
    ("PropertySignature#8", "interface I {a?}", "interface I {\n    a?;\n}", false),
    ("PropertySignature#9", "interface I {a: b}", "interface I {\n    a: b;\n}", false),
    ("MethodSignature#1", "interface I {a()}", "interface I {\n    a();\n}", false),
    ("MethodSignature#2", "interface I {\"a\"()}", "interface I {\n    \"a\"();\n}", false),
    ("MethodSignature#3", "interface I {'a'()}", "interface I {\n    'a'();\n}", false),
    ("MethodSignature#4", "interface I {0()}", "interface I {\n    0();\n}", false),
    ("MethodSignature#5", "interface I {0n()}", "interface I {\n    0n();\n}", false),
    ("MethodSignature#6", "interface I {[a]()}", "interface I {\n    [a]();\n}", false),
    ("MethodSignature#7", "interface I {a?()}", "interface I {\n    a?();\n}", false),
    ("MethodSignature#8", "interface I {a<T>()}", "interface I {\n    a<T>();\n}", false),
    ("MethodSignature#9", "interface I {a(): b}", "interface I {\n    a(): b;\n}", false),
    ("MethodSignature#10", "interface I {a(b): c}", "interface I {\n    a(b): c;\n}", false),
    ("CallSignature#1", "interface I {()}", "interface I {\n    ();\n}", false),
    ("CallSignature#2", "interface I {():a}", "interface I {\n    (): a;\n}", false),
    ("CallSignature#3", "interface I {(p)}", "interface I {\n    (p);\n}", false),
    ("CallSignature#4", "interface I {<T>()}", "interface I {\n    <T>();\n}", false),
    ("ConstructSignature#1", "interface I {new ()}", "interface I {\n    new ();\n}", false),
    ("ConstructSignature#2", "interface I {new ():a}", "interface I {\n    new (): a;\n}", false),
    ("ConstructSignature#3", "interface I {new (p)}", "interface I {\n    new (p);\n}", false),
    ("ConstructSignature#4", "interface I {new <T>()}", "interface I {\n    new <T>();\n}", false),
    ("IndexSignatureDeclaration#1", "interface I {[a]}", "interface I {\n    [a];\n}", false),
    ("IndexSignatureDeclaration#2", "interface I {[a: b]}", "interface I {\n    [a: b];\n}", false),
    ("IndexSignatureDeclaration#3", "interface I {[a: b]: c}", "interface I {\n    [a: b]: c;\n}", false),
    ("PropertyDeclaration#1", "class C {a}", "class C {\n    a;\n}", false),
    ("PropertyDeclaration#2", "class C {readonly a}", "class C {\n    readonly a;\n}", false),
    ("PropertyDeclaration#3", "class C {static a}", "class C {\n    static a;\n}", false),
    ("PropertyDeclaration#4", "class C {accessor a}", "class C {\n    accessor a;\n}", false),
    ("PropertyDeclaration#5", "class C {\"a\"}", "class C {\n    \"a\";\n}", false),
    ("PropertyDeclaration#6", "class C {'a'}", "class C {\n    'a';\n}", false),
    ("PropertyDeclaration#7", "class C {0}", "class C {\n    0;\n}", false),
    ("PropertyDeclaration#8", "class C {0n}", "class C {\n    0n;\n}", false),
    ("PropertyDeclaration#9", "class C {[a]}", "class C {\n    [a];\n}", false),
    ("PropertyDeclaration#10", "class C {#a}", "class C {\n    #a;\n}", false),
    ("PropertyDeclaration#11", "class C {a?}", "class C {\n    a?;\n}", false),
    ("PropertyDeclaration#12", "class C {a!}", "class C {\n    a!;\n}", false),
    ("PropertyDeclaration#13", "class C {a: b}", "class C {\n    a: b;\n}", false),
    ("PropertyDeclaration#14", "class C {a = b}", "class C {\n    a = b;\n}", false),
    ("PropertyDeclaration#15", "class C {@a b}", "class C {\n    @a\n    b;\n}", false),
    ("MethodDeclaration#1", "class C {a()}", "class C {\n    a();\n}", false),
    ("MethodDeclaration#2", "class C {\"a\"()}", "class C {\n    \"a\"();\n}", false),
    ("MethodDeclaration#3", "class C {'a'()}", "class C {\n    'a'();\n}", false),
    ("MethodDeclaration#4", "class C {0()}", "class C {\n    0();\n}", false),
    ("MethodDeclaration#5", "class C {0n()}", "class C {\n    0n();\n}", false),
    ("MethodDeclaration#6", "class C {[a]()}", "class C {\n    [a]();\n}", false),
    ("MethodDeclaration#7", "class C {#a()}", "class C {\n    #a();\n}", false),
    ("MethodDeclaration#8", "class C {a?()}", "class C {\n    a?();\n}", false),
    ("MethodDeclaration#9", "class C {a<T>()}", "class C {\n    a<T>();\n}", false),
    ("MethodDeclaration#10", "class C {a(): b}", "class C {\n    a(): b;\n}", false),
    ("MethodDeclaration#11", "class C {a(b): c}", "class C {\n    a(b): c;\n}", false),
    ("MethodDeclaration#12", "class C {a() {} }", "class C {\n    a() { }\n}", false),
    ("MethodDeclaration#13", "class C {@a b() {} }", "class C {\n    @a\n    b() { }\n}", false),
    ("MethodDeclaration#14", "class C {static a() {} }", "class C {\n    static a() { }\n}", false),
    ("MethodDeclaration#15", "class C {async a() {} }", "class C {\n    async a() { }\n}", false),
    ("GetAccessorDeclaration#1", "class C {get a()}", "class C {\n    get a();\n}", false),
    ("GetAccessorDeclaration#2", "class C {get \"a\"()}", "class C {\n    get \"a\"();\n}", false),
    ("GetAccessorDeclaration#3", "class C {get 'a'()}", "class C {\n    get 'a'();\n}", false),
    ("GetAccessorDeclaration#4", "class C {get 0()}", "class C {\n    get 0();\n}", false),
    ("GetAccessorDeclaration#5", "class C {get 0n()}", "class C {\n    get 0n();\n}", false),
    ("GetAccessorDeclaration#6", "class C {get [a]()}", "class C {\n    get [a]();\n}", false),
    ("GetAccessorDeclaration#7", "class C {get #a()}", "class C {\n    get #a();\n}", false),
    ("GetAccessorDeclaration#8", "class C {get a(): b}", "class C {\n    get a(): b;\n}", false),
    ("GetAccessorDeclaration#9", "class C {get a(b): c}", "class C {\n    get a(b): c;\n}", false),
    ("GetAccessorDeclaration#10", "class C {get a() {} }", "class C {\n    get a() { }\n}", false),
    ("GetAccessorDeclaration#11", "class C {@a get b() {} }", "class C {\n    @a\n    get b() { }\n}", false),
    ("GetAccessorDeclaration#12", "class C {static get a() {} }", "class C {\n    static get a() { }\n}", false),
    ("SetAccessorDeclaration#1", "class C {set a()}", "class C {\n    set a();\n}", false),
    ("SetAccessorDeclaration#2", "class C {set \"a\"()}", "class C {\n    set \"a\"();\n}", false),
    ("SetAccessorDeclaration#3", "class C {set 'a'()}", "class C {\n    set 'a'();\n}", false),
    ("SetAccessorDeclaration#4", "class C {set 0()}", "class C {\n    set 0();\n}", false),
    ("SetAccessorDeclaration#5", "class C {set 0n()}", "class C {\n    set 0n();\n}", false),
    ("SetAccessorDeclaration#6", "class C {set [a]()}", "class C {\n    set [a]();\n}", false),
    ("SetAccessorDeclaration#7", "class C {set #a()}", "class C {\n    set #a();\n}", false),
    ("SetAccessorDeclaration#8", "class C {set a(): b}", "class C {\n    set a(): b;\n}", false),
    ("SetAccessorDeclaration#9", "class C {set a(b): c}", "class C {\n    set a(b): c;\n}", false),
    ("SetAccessorDeclaration#10", "class C {set a() {} }", "class C {\n    set a() { }\n}", false),
    ("SetAccessorDeclaration#11", "class C {@a set b() {} }", "class C {\n    @a\n    set b() { }\n}", false),
    ("SetAccessorDeclaration#12", "class C {static set a() {} }", "class C {\n    static set a() { }\n}", false),
    ("ConstructorDeclaration#1", "class C {constructor()}", "class C {\n    constructor();\n}", false),
    ("ConstructorDeclaration#2", "class C {constructor(): b}", "class C {\n    constructor(): b;\n}", false),
    ("ConstructorDeclaration#3", "class C {constructor(b): c}", "class C {\n    constructor(b): c;\n}", false),
    ("ConstructorDeclaration#4", "class C {constructor() {} }", "class C {\n    constructor() { }\n}", false),
    ("ConstructorDeclaration#5", "class C {@a constructor() {} }", "class C {\n    constructor() { }\n}", false),
    ("ConstructorDeclaration#6", "class C {private constructor() {} }", "class C {\n    private constructor() { }\n}", false),
    ("ClassStaticBlockDeclaration", "class C {static { }}", "class C {\n    static { }\n}", false),
    ("SemicolonClassElement#1", "class C {;}", "class C {\n    ;\n}", false),
    ("ParameterDeclaration#1", "function f(a)", "function f(a);", false),
    ("ParameterDeclaration#2", "function f(a: b)", "function f(a: b);", false),
    ("ParameterDeclaration#3", "function f(a = b)", "function f(a = b);", false),
    ("ParameterDeclaration#4", "function f(a?)", "function f(a?);", false),
    ("ParameterDeclaration#5", "function f(...a)", "function f(...a);", false),
    ("ParameterDeclaration#6", "function f(this)", "function f(this);", false),
    ("ObjectBindingPattern#1", "function f({})", "function f({});", false),
    ("ObjectBindingPattern#2", "function f({a})", "function f({ a });", false),
    ("ObjectBindingPattern#3", "function f({a = b})", "function f({ a = b });", false),
    ("ObjectBindingPattern#4", "function f({a: b})", "function f({ a: b });", false),
    ("ObjectBindingPattern#5", "function f({a: b = c})", "function f({ a: b = c });", false),
    ("ObjectBindingPattern#6", "function f({\"a\": b})", "function f({ \"a\": b });", false),
    ("ObjectBindingPattern#7", "function f({'a': b})", "function f({ 'a': b });", false),
    ("ObjectBindingPattern#8", "function f({0: b})", "function f({ 0: b });", false),
    ("ObjectBindingPattern#9", "function f({[a]: b})", "function f({ [a]: b });", false),
    ("ObjectBindingPattern#10", "function f({...a})", "function f({ ...a });", false),
    ("ObjectBindingPattern#11", "function f({a: {}})", "function f({ a: {} });", false),
    ("ObjectBindingPattern#12", "function f({a: []})", "function f({ a: [] });", false),
    ("ArrayBindingPattern#1", "function f([])", "function f([]);", false),
    ("ArrayBindingPattern#2", "function f([,])", "function f([,]);", false),
    ("ArrayBindingPattern#3", "function f([a])", "function f([a]);", false),
    ("ArrayBindingPattern#4", "function f([a, b])", "function f([a, b]);", false),
    ("ArrayBindingPattern#5", "function f([a, , b])", "function f([a, , b]);", false),
    ("ArrayBindingPattern#6", "function f([a = b])", "function f([a = b]);", false),
    ("ArrayBindingPattern#7", "function f([...a])", "function f([...a]);", false),
    ("ArrayBindingPattern#8", "function f([{}])", "function f([{}]);", false),
    ("ArrayBindingPattern#9", "function f([[]])", "function f([[]]);", false),
    ("TypeParameterDeclaration#1", "function f<T>();", "function f<T>();", false),
    ("TypeParameterDeclaration#2", "function f<in T>();", "function f<in T>();", false),
    ("TypeParameterDeclaration#3", "function f<T extends U>();", "function f<T extends U>();", false),
    ("TypeParameterDeclaration#4", "function f<T = U>();", "function f<T = U>();", false),
    ("TypeParameterDeclaration#5", "function f<T extends U = V>();", "function f<T extends U = V>();", false),
    ("TypeParameterDeclaration#6", "function f<T, U>();", "function f<T, U>();", false),
    ("JsxElement1", "<a></a>", "<a></a>;", true),
    ("JsxElement2", "<this></this>", "<this></this>;", true),
    ("JsxElement3", "<a:b></a:b>", "<a:b></a:b>;", true),
    ("JsxElement4", "<a.b></a.b>", "<a.b></a.b>;", true),
    ("JsxElement5", "<a<b>></a>", "<a<b>></a>;", true),
    ("JsxElement6", "<a b></a>", "<a b></a>;", true),
    ("JsxElement7", "<a>b</a>", "<a>b</a>;", true),
    ("JsxElement8", "<a>{b}</a>", "<a>{b}</a>;", true),
    ("JsxElement9", "<a><b></b></a>", "<a><b></b></a>;", true),
    ("JsxElement10", "<a><b /></a>", "<a><b /></a>;", true),
    ("JsxElement11", "<a><></></a>", "<a><></></a>;", true),
    ("JsxElement12", "<a>\n    {/* missing */}\n    {\n        // foo\n    }\n</a>", "<a>\n    {/* missing */}\n    {\n    // foo\n    }\n</a>;", true),
    ("JsxSelfClosingElement1", "<a />", "<a />;", true),
    ("JsxSelfClosingElement2", "<this />", "<this />;", true),
    ("JsxSelfClosingElement3", "<a:b />", "<a:b />;", true),
    ("JsxSelfClosingElement4", "<a.b />", "<a.b />;", true),
    ("JsxSelfClosingElement5", "<a<b> />", "<a<b> />;", true),
    ("JsxSelfClosingElement6", "<a b/>", "<a b/>;", true),
    ("JsxFragment1", "<></>", "<></>;", true),
    ("JsxFragment2", "<>b</>", "<>b</>;", true),
    ("JsxFragment3", "<>{b}</>", "<>{b}</>;", true),
    ("JsxFragment4", "<><b></b></>", "<><b></b></>;", true),
    ("JsxFragment5", "<><b /></>", "<><b /></>;", true),
    ("JsxFragment6", "<><></></>", "<><></></>;", true),
    ("JsxAttribute1", "<a b/>", "<a b/>;", true),
    ("JsxAttribute2", "<a b:c/>", "<a b:c/>;", true),
    ("JsxAttribute3", "<a b=\"c\"/>", "<a b=\"c\"/>;", true),
    ("JsxAttribute4", "<a b='c'/>", "<a b='c'/>;", true),
    ("JsxAttribute5", "<a b={c}/>", "<a b={c}/>;", true),
    ("JsxAttribute6", "<a b=<c></c>/>", "<a b=<c></c>/>;", true),
    ("JsxAttribute7", "<a b=<c />/>", "<a b=<c />/>;", true),
    ("JsxAttribute8", "<a b=<></>/>", "<a b=<></>/>;", true),
    ("JsxSpreadAttribute", "<a {...b}/>", "<a {...b}/>;", true),
];

#[test]
fn test_emit() {
    let mut failures = Vec::new();
    for &(title, input, output, jsx) in EMIT_CASES {
        let file = parse_type_script(input, jsx);
        if !file.diagnostics().is_empty() {
            failures.push(format!("{}: input has parse diagnostics", title));
            continue;
        }
        if let Err(e) = check_emit(None, file, output) {
            failures.push(format!("{}: {}", title, e));
        }
    }
    assert!(failures.is_empty(), "{} of {} cases failed:\n{}", failures.len(), EMIT_CASES.len(), failures.join("\n"));
}

#[test]
fn test_parenthesize_decorator() {
    let f = NodeFactory::default();
    let a = f.new_identifier("a");
    let plus = f.new_token(Kind::PlusToken);
    let b = f.new_identifier("b");
    let bin = f.new_binary_expression(None, a, None, plus, b);
    let decorator = f.new_decorator(bin);
    let modifiers = f.new_modifier_list(vec![decorator]);
    let name = f.new_identifier("C");
    let members = f.new_node_list(vec![]);
    let class = f.new_class_declaration(Some(modifiers), Some(name), None, None, members);
    let file = synthetic_source_file(&f, vec![class]);
    check_emit(None, file, "@(a + b)\nclass C {\n}").unwrap();
}

#[test]
fn test_parenthesize_computed_property_name() {
    let f = NodeFactory::default();
    let a = f.new_identifier("a");
    let comma = f.new_token(Kind::CommaToken);
    let b = f.new_identifier("b");
    // will be parenthesized on emit:
    let bin = f.new_binary_expression(None, a, None, comma, b);
    let computed = f.new_computed_property_name(bin);
    let property = f.new_property_declaration(None, computed, None, None, None);
    let members = f.new_node_list(vec![property]);
    let name = f.new_identifier("C");
    let class = f.new_class_declaration(None, Some(name), None, None, members);
    let file = synthetic_source_file(&f, vec![class]);
    check_emit(None, file, "class C {\n    [(a, b)];\n}").unwrap();
}

#[test]
fn test_parenthesize_array_literal() {
    let f = NodeFactory::default();
    let a = f.new_identifier("a");
    let comma = f.new_token(Kind::CommaToken);
    let b = f.new_identifier("b");
    // will be parenthesized on emit:
    let bin = f.new_binary_expression(None, a, None, comma, b);
    let elements = f.new_node_list(vec![bin]);
    let array = f.new_array_literal_expression(elements, false /*multiLine*/);
    let statement = f.new_expression_statement(array);
    let file = synthetic_source_file(&f, vec![statement]);
    check_emit(None, file, "[(a, b)];").unwrap();
}

#[test]
fn test_parenthesize_binary_expression_mixing_nullish_coalescing() {
    let tests: &[(&str, Kind, Kind, &str, &str)] = &[
        // inner ?? on left side of || or &&
        ("BarBarWithLeftQuestionQuestion", Kind::QuestionQuestionToken, Kind::BarBarToken, "left", "(a ?? b) || c;"),
        ("AmpersandAmpersandWithLeftQuestionQuestion", Kind::QuestionQuestionToken, Kind::AmpersandAmpersandToken, "left", "(a ?? b) && c;"),
        // inner ?? on right side of || or &&
        ("BarBarWithRightQuestionQuestion", Kind::QuestionQuestionToken, Kind::BarBarToken, "right", "a || (b ?? c);"),
        ("AmpersandAmpersandWithRightQuestionQuestion", Kind::QuestionQuestionToken, Kind::AmpersandAmpersandToken, "right", "a && (b ?? c);"),
        // inner || or && on left side of ??
        ("QuestionQuestionWithLeftBarBar", Kind::BarBarToken, Kind::QuestionQuestionToken, "left", "(a || b) ?? c;"),
        ("QuestionQuestionWithLeftAmpersandAmpersand", Kind::AmpersandAmpersandToken, Kind::QuestionQuestionToken, "left", "(a && b) ?? c;"),
        // inner || or && on right side of ??
        ("QuestionQuestionWithRightBarBar", Kind::BarBarToken, Kind::QuestionQuestionToken, "right", "a ?? (b || c);"),
        ("QuestionQuestionWithRightAmpersandAmpersand", Kind::AmpersandAmpersandToken, Kind::QuestionQuestionToken, "right", "a ?? (b && c);"),
    ];
    for &(title, inner_op, outer_op, side, output) in tests {
        let f = NodeFactory::default();
        let outer_expr = if side == "left" {
            let a = f.new_identifier("a");
            let inner_token = f.new_token(inner_op);
            let b = f.new_identifier("b");
            let inner_expr = f.new_binary_expression(None, a, None, inner_token, b);
            let outer_token = f.new_token(outer_op);
            let c = f.new_identifier("c");
            f.new_binary_expression(None, inner_expr /*left: (a innerOp b)*/, None, outer_token, c)
        } else {
            // Go builds the inner expression over `a`/`b` and then replaces its operands with `b`/`c`.
            let b = f.new_identifier("b");
            let inner_token = f.new_token(inner_op);
            let c = f.new_identifier("c");
            let inner_expr = f.new_binary_expression(None, b, None, inner_token, c);
            let a = f.new_identifier("a");
            let outer_token = f.new_token(outer_op);
            f.new_binary_expression(None, a, None, outer_token, inner_expr /*right: (b innerOp c)*/)
        };
        let statement = f.new_expression_statement(outer_expr);
        let file = synthetic_source_file(&f, vec![statement]);
        if let Err(e) = check_emit(None, file, output) {
            panic!("{}: {}", title, e);
        }
    }
}

#[test]
fn test_name_generation() {
    let ec = new_emit_context();
    let file = {
        let f = &ec.factory;
        let temp1 = f.new_temp_variable();
        let decl1 = f.new_variable_declaration(temp1, None, None, None);
        let decls1 = f.new_node_list(vec![decl1]);
        let list1 = f.new_variable_declaration_list(decls1, NodeFlags::None);
        let statement1 = f.new_variable_statement(None, list1);
        let temp2 = f.new_temp_variable();
        let decl2 = f.new_variable_declaration(temp2, None, None, None);
        let decls2 = f.new_node_list(vec![decl2]);
        let list2 = f.new_variable_declaration_list(decls2, NodeFlags::None);
        let statement2 = f.new_variable_statement(None, list2);
        let body_statements = f.new_node_list(vec![statement2]);
        let body = f.new_block(body_statements, true);
        let name = f.new_identifier("f");
        let parameters = f.new_node_list(vec![]);
        let function = f.new_function_declaration(None, None, Some(name), None, Some(parameters), None, None, Some(body));
        synthetic_source_file(f.as_node_factory(), vec![statement1, function])
    };
    check_emit(Some(ec), file, "var _a;\nfunction f() {\n    var _a;\n}").unwrap();
}

#[test]
fn test_omit_trailing_semicolon() {
    let f = NodeFactory::default();
    let name = f.new_identifier("m");
    let parameters = f.new_node_list(vec![]);
    let void = f.new_keyword_type_node(Kind::VoidKeyword);
    let method_signature = f.new_method_signature_declaration(None, name, None, None, Some(parameters), Some(void));
    let file = parse_type_script("interface I {}", false);

    let mut default_printer = new_printer(PrinterOptions { new_line: NewLineKind::LF, ..Default::default() }, PrintHandlers::default(), None);
    assert_eq!(default_printer.emit(method_signature, Some(file)), "m(): void;");

    let mut omit_printer = new_printer(PrinterOptions { new_line: NewLineKind::LF, omit_trailing_semicolon: true, ..Default::default() }, PrintHandlers::default(), None);
    assert_eq!(omit_printer.emit(method_signature, Some(file)), "m(): void");

    let for_file = parse_type_script("for (;;) {}", false);
    let got = omit_printer.emit_source_file(for_file);
    assert_eq!(got.strip_suffix('\n').unwrap_or(&got), "for (;;) { }");
}

#[test]
fn test_writers_used_by_checker() {
    // The checker prints type nodes through `new_text_writer("", 0)` and `get_single_line_string_writer()`.
    let ec = new_emit_context();
    let node = {
        let f = &ec.factory;
        let a = f.new_keyword_type_node(Kind::StringKeyword);
        let b = f.new_keyword_type_node(Kind::NumberKeyword);
        let types = f.new_node_list(vec![a, b]);
        f.new_union_type_node(types)
    };
    let mut p = new_printer(PrinterOptions { remove_comments: true, ..Default::default() }, PrintHandlers::default(), Some(ec));
    let mut writer = new_text_writer("", 0);
    p.write(node, None, &mut writer, None);
    assert_eq!(writer.string(), "string | number");
    let (mut writer, put_writer) = get_single_line_string_writer();
    p.write(node, None, &mut *writer, None);
    assert_eq!(writer.string(), "string | number");
    put_writer();
}

#[test]
fn test_synthetic_comments_with_source_file() {
    // nodebuilderimpl.go attaches "/*unresolved*/" and "/*elided*/" comments; they print only with comments enabled and
    // a current source file (checker printer.go uses the default printer for the unresolved type).
    let ec = new_emit_context();
    let file = parse_type_script("let x;", false);
    let any = {
        let f = &ec.factory;
        f.new_keyword_type_node(Kind::AnyKeyword)
    };
    ec.add_synthetic_leading_comment(any, Kind::MultiLineCommentTrivia, "unresolved", false /*hasTrailingNewLine*/);
    let mut p = new_printer(PrinterOptions::default(), PrintHandlers::default(), Some(ec));
    let mut writer = new_text_writer("", 0);
    p.write(any, Some(file), &mut writer, None);
    assert_eq!(writer.string(), "/*unresolved*/ any");
    p.write(any, None, &mut writer, None);
    assert_eq!(writer.string(), "any");

    let literal = {
        let f = &ec.factory;
        let name = f.new_identifier("a");
        let type_node = f.new_keyword_type_node(Kind::StringKeyword);
        let member = f.new_property_signature_declaration(None, name, None, Some(type_node), None);
        let members = f.new_node_list(vec![member]);
        f.new_type_literal_node(members)
    };
    let member = literal.as_type_literal_node().members.nodes()[0];
    ec.add_synthetic_trailing_comment(member, Kind::MultiLineCommentTrivia, "elided", false /*hasTrailingNewLine*/);
    ec.set_emit_flags(literal, EmitFlags::SingleLine);
    p.write(literal, Some(file), &mut writer, None);
    assert_eq!(writer.string(), "{ a: string; /*elided*/ }");
}

#[test]
fn test_node_visitor_environment_hooks() {
    // Regression: the EmitContext visitor hooks (emitcontext.go VisitParameters/VisitFunctionBody) must move parameter
    // initializers and binding patterns into the body when a temp is hoisted while visiting the parameter list, and the
    // hooks must be re-entrant (a RefCell borrow held across the visit callback panics here).
    let ec = new_emit_context();
    let file = parse_type_script("function f(a = 1, { b } = {}) {\n    return a;\n}", false);
    let visit: VisitFn = std::rc::Rc::new(move |v, n| {
        if n.kind() == Kind::NumericLiteral {
            ec.add_variable_declaration(ec.factory.new_temp_variable());
            return Some(n);
        }
        v.visit_each_child(Some(n))
    });
    let mut visitor = ec.new_node_visitor(visit);
    let visited = visitor.visit_source_file(file.as_node());
    let mut printer = new_printer(PrinterOptions { new_line: NewLineKind::LF, ..Default::default() }, PrintHandlers::default(), Some(ec));
    let text = printer.emit_source_file(visited.as_source_file_p());
    assert_eq!(text, "function f(a, _a) {\n    var _b;\n    if (a === void 0) { a = 1; }\n    var { b } = _a === void 0 ? {} : _a;\n    return a;\n}\n");
}
