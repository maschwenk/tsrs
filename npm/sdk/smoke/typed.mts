// Type-only check of the published declarations from a nodenext consumer: compiled with `tsrs -p` in the temp
// consumer (see ../smoke-consumer.mjs), never run.
import { typescriptCommit, version } from "tsrs";
import { API as AsyncAPI } from "tsrs/unstable/async";
import { type Node, type SourceFile, SyntaxKind } from "tsrs/unstable/ast";
import { isIdentifier } from "tsrs/unstable/ast/is";
import { createFileSystem, type FileSystemCallbacks } from "tsrs/unstable/fs";
import { API, type APIOptions, type Diagnostic, type Snapshot } from "tsrs/unstable/sync";

declare const callbacks: FileSystemCallbacks;
const options: APIOptions = { cwd: "/", tsserverPath: "/path/to/tsrs", fs: callbacks };
const requestFileSystem = createFileSystem([["/a.ts", "export {};"]]);
const api: API = new API(options);
const snapshot: Snapshot = api.createSnapshot({ openProject: "/tsconfig.json" });
const sourceFile: SourceFile | undefined = snapshot.getConfiguredProject("/tsconfig.json")?.program.getSourceFile("/a.ts");
const diagnostics: readonly Diagnostic[] = snapshot.getConfiguredProject("/tsconfig.json")!.program.getSemanticDiagnostics("/a.ts");
const first: Node | undefined = sourceFile?.statements[0];
const isId: boolean = first !== undefined && isIdentifier(first);
const kind: SyntaxKind = SyntaxKind.Identifier;
const asyncApi = new AsyncAPI({ cwd: "/" });
const closed: Promise<void> = asyncApi.close();
const strings: string[] = [version, typescriptCommit];
export { closed, requestFileSystem, diagnostics, isId, kind, strings };
