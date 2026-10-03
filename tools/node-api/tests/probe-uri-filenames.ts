// Standalone probe (not a test file): fileNames/rootFiles of the inferred project created for the four VS Code
// document URIs of the upstream test "full file system accepts and caches paths decoded from VS Code document
// URIs", for one input order. Prints the raw createSnapshot and getDefaultProjectForFile responses' file lists.
//
//   node --conditions @typescript/source test/parity/probe-uri-filenames.ts <binary> <sync|async> <order: e.g. 0123>

import { API as AsyncAPI } from "@typescript/typescript/unstable/async";
import { createFileSystem } from "@typescript/typescript/unstable/fs";
import { API } from "@typescript/typescript/unstable/sync";

const [binary, mode, order = "0123"] = process.argv.slice(2);
const docs: [{ uri: string; }, string][] = [
    [{ uri: "file:///workspace/file%20name.ts" }, `export const file = true;`],
    [{ uri: "vscode-remote://ssh-remote+host/workspace/src/remote%20name.ts" }, `export const remote = true;`],
    [{ uri: "vscode-notebook-cell:/workspace/notebook.ipynb/cell%20name.ts" }, `export const cell = true;`],
    [{ uri: "untitled:Untitled-1" }, `export const untitled = true;`],
];
const files = [...order].map(i => docs[Number(i)]);
const params = { openFiles: files.map(([d]) => d), fileSystem: createFileSystem(files) };
const lists = (projects: any[]) => projects.map(p => ({ id: p.id, rootFiles: p.rootFiles, fileNames: p.parsedCommandLine?.fileNames?.filter((f: string) => !f.includes("lib.")) }));
let out: unknown;
if (mode === "async") {
    const api = new AsyncAPI({ cwd: "/", tsserverPath: binary });
    await api.parseCommandLine([]);
    const client = (api as any).client;
    const snap = await client.apiRequest("createSnapshot", params);
    const perDoc = [];
    for (const [d] of files) perDoc.push(lists([await client.apiRequest("getDefaultProjectForFile", { snapshot: snap.snapshot, file: d })].filter(Boolean)));
    out = { snapshot: lists(snap.projects), perDoc };
    await api.close();
}
else {
    const api = new API({ cwd: "/", tsserverPath: binary });
    api.parseCommandLine([]);
    const client = (api as any).client;
    const snap = client.apiRequest("createSnapshot", params);
    const perDoc = files.map(([d]) => lists([client.apiRequest("getDefaultProjectForFile", { snapshot: snap.snapshot, file: d })].filter(Boolean)));
    out = { snapshot: lists(snap.projects), perDoc };
    api.close();
}
console.log(JSON.stringify({ mode, order, ...(out as object) }));
