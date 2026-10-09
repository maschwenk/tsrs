#!/usr/bin/env node

// Generates crates/tsrs_lsproto/src/lsp_generated.rs from the LSP meta model (docs/LSP.md, "Protocol types
// and JSON"). Port of ts-ref/tsc/internal/lsp/lsproto/_generate/generate.mts: the model processing (custom
// structures, enumerations, requests and notifications, model patches, inheritance flattening, notebook
// removal, type resolution, union naming, discriminator and presence dispatch selection) is Go's, verbatim;
// only the Go emitter is replaced by a Rust emitter.
//
// Usage (from the repository root, offline):
//   node tools/gen-lsproto/generate.mts     regenerates crates/tsrs_lsproto/src/lsp_generated.rs
//   node tools/gen-lsproto/fetchModel.mts   refreshes metaModel.json / metaModelSchema.mts (network)
//   python3 tools/gen-lsproto/check.py      checks every Go lsp_generated.go name has a Rust counterpart

import fs from "node:fs";
import path from "node:path";
import url from "node:url";
import type {
    Enumeration,
    MetaModel,
    Notification,
    OrType,
    Property,
    ReferenceType,
    Request,
    Structure,
    Type,
    TypeAlias,
} from "./metaModelSchema.mts";

const __filename = url.fileURLToPath(new URL(import.meta.url));
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, "../..");

const out = path.resolve(repoRoot, "crates/tsrs_lsproto/src/lsp_generated.rs");
const metaModelPath = path.resolve(__dirname, "metaModel.json");

if (!fs.existsSync(metaModelPath)) {
    console.error("Meta model file not found; did you forget to run fetchModel.mts?");
    process.exit(1);
}

const model: MetaModel = JSON.parse(fs.readFileSync(metaModelPath, "utf-8"));

// Custom structures to add to the model
const customStructures: Structure[] = [
    {
        name: "InitializationOptions",
        properties: [
            {
                name: "disablePushDiagnostics",
                type: { kind: "base", name: "boolean" },
                optional: true,
                documentation: "DisablePushDiagnostics disables automatic pushing of diagnostics to the client.",
            },
            {
                name: "codeLensShowLocationsCommandName",
                type: { kind: "base", name: "string" },
                optional: true,
                documentation: "The client-side command name that resolved references/implementations `CodeLens` should trigger. Arguments passed will be `(DocumentUri, Position, Location[])`.",
            },
            {
                name: "userPreferences",
                type: { kind: "reference", name: "any" },
                optional: true,
                documentation: "userPreferences and/or formatting options if provided at initialization.",
            },
            {
                name: "enableTelemetry",
                type: { kind: "base", name: "boolean" },
                optional: true,
                documentation: "EnableTelemetry enables sending telemetry events from the server to the client.",
            },
            {
                name: "logVerbosity",
                type: { kind: "reference", name: "LogVerbosity" },
                optional: true,
                documentation: "The initial log verbosity level, matching the client's output channel log level at startup. Subsequent changes are sent via custom/setLogVerbosity.",
            },
            {
                name: "runExternalCode",
                type: { kind: "base", name: "boolean" },
                optional: true,
                documentation: "RunExternalCode allows configured content mappers to launch external plugin processes. The client should set this only for trusted workspaces. It mirrors the --runExternalCode CLI flag.",
            },
            {
                name: "trackFlakyDiagnostics",
                type: { kind: "reference", name: "DiagnosticFlakeLogLevel" },
                optional: true,
                documentation: "The level at which we track flaky diagnostics, if at all.",
            },
        ],
        documentation: "InitializationOptions contains user-provided initialization options.",
    },
    {
        name: "AutoImportFix",
        properties: [
            {
                name: "kind",
                type: { kind: "reference", name: "AutoImportFixKind" },
                omitzeroValue: true,
            },
            {
                name: "name",
                type: { kind: "base", name: "string" },
                omitzeroValue: true,
            },
            {
                name: "importKind",
                type: { kind: "reference", name: "ImportKind" },
            },
            {
                name: "useRequire",
                type: { kind: "base", name: "boolean" },
                omitzeroValue: true,
            },
            {
                name: "addAsTypeOnly",
                type: { kind: "reference", name: "AddAsTypeOnly" },
            },
            {
                name: "moduleSpecifier",
                type: { kind: "base", name: "string" },
                documentation: "The module specifier for this auto-import.",
                omitzeroValue: true,
            },
            {
                name: "importIndex",
                type: { kind: "base", name: "integer" },
                documentation: "Index of the import to modify when adding to an existing import declaration.",
            },
            {
                name: "usagePosition",
                type: { kind: "reference", name: "Position" },
                optional: true,
            },
            {
                name: "namespacePrefix",
                type: { kind: "base", name: "string" },
                omitzeroValue: true,
            },
        ],
        documentation: "AutoImportFix contains information about an auto-import suggestion.",
    },
    {
        name: "CompletionItemData",
        properties: [
            {
                name: "fileName",
                type: { kind: "base", name: "string" },
                documentation: "The file name where the completion was requested.",
                omitzeroValue: true,
            },
            {
                name: "position",
                type: { kind: "base", name: "integer" },
                documentation: "The position where the completion was requested.",
                omitzeroValue: true,
            },
            {
                name: "supplementalFileIndex",
                type: { kind: "base", name: "integer" },
                optional: true,
                documentation: "Zero-based index into the canonical file's supplemental source files. Absent when the completion was requested in the canonical file.",
            },
            {
                name: "source",
                type: { kind: "base", name: "string" },
                documentation: "Special source value for disambiguation.",
                omitzeroValue: true,
            },
            {
                name: "name",
                type: { kind: "base", name: "string" },
                documentation: "The name of the completion item.",
                omitzeroValue: true,
            },
            {
                name: "autoImport",
                type: { kind: "reference", name: "AutoImportFix" },
                optional: true,
                documentation: "Auto-import data for this completion item.",
            },
            {
                name: "isImportStatementCompletion",
                type: { kind: "base", name: "boolean" },
                omitzeroValue: true,
            },
        ],
        documentation: "CompletionItemData is preserved on a CompletionItem between CompletionRequest and CompletionResolveRequest.",
    },
    {
        name: "CodeLensData",
        properties: [
            {
                name: "kind",
                type: { kind: "reference", name: "CodeLensKind" },
                documentation: `The kind of the code lens ("references" or "implementations").`,
            },
            {
                name: "uri",
                type: { kind: "base", name: "DocumentUri" },
                documentation: `The document in which the code lens and its range are located.`,
            },
            {
                name: "position",
                type: { kind: "base", name: "integer" },
                documentation: `The position of the code lens declaration in its virtual source file.`,
            },
            {
                name: "supplementalFileIndex",
                type: { kind: "base", name: "integer" },
                optional: true,
                documentation: `Zero-based index into the canonical file's supplemental source files. Absent for the canonical source file.`,
            },
        ],
    },
    {
        name: "ExperimentalServerCapabilities",
        properties: [
            {
                name: "customSourceDefinitionProvider",
                type: { kind: "base", name: "boolean" },
                optional: true,
                documentation: "The server provides source definition support via custom/textDocument/sourceDefinition.",
            },
            {
                name: "customMultiDocumentHighlightProvider",
                type: { kind: "base", name: "boolean" },
                optional: true,
                documentation: "The server provides multi-document highlight support via custom/textDocument/multiDocumentHighlight.",
            },
        ],
        documentation: "ExperimentalServerCapabilities contains experimental capabilities under development.",
    },
    {
        name: "ExperimentalClientCapabilities",
        properties: [
            {
                name: "hoverVerbosityLevel",
                type: { kind: "base", name: "boolean" },
                optional: true,
                documentation: "The client supports hover verbosityLevel requests and canIncreaseVerbosity responses.",
            },
        ],
        documentation: "ExperimentalClientCapabilities contains experimental capabilities under development.",
    },
    {
        name: "VSOnAutoInsertOptions",
        properties: [
            {
                name: "_vs_triggerCharacters",
                type: { kind: "array", element: { kind: "base", name: "string" } },
                documentation: "List of trigger characters that trigger auto-insert.",
            },
        ],
        documentation: "Options for the textDocument/_vs_onAutoInsert provider capability.",
    },
    {
        name: "VSReferenceItem",
        properties: [
            {
                name: "_vs_id",
                type: { kind: "base", name: "integer" },
                documentation: "Unique identifier for this reference item.",
            },
            {
                name: "_vs_definitionId",
                type: { kind: "base", name: "integer" },
                optional: true,
                documentation: "The ID of the definition item this reference belongs to. Absent for definition items themselves.",
            },
            {
                name: "_vs_kind",
                type: { kind: "array", element: { kind: "reference", name: "VSReferenceKind" } },
                optional: true,
                documentation: "The kind(s) of this reference (read, write, etc.).",
            },
            {
                name: "_vs_location",
                type: { kind: "reference", name: "Location" },
                documentation: "The location of this reference.",
            },
            {
                name: "_vs_definitionText",
                type: { kind: "reference", name: "VSClassifiedTextElement" },
                optional: true,
                documentation: "Classified display text for the definition (used for grouping headers in the UI).",
            },
            {
                name: "_vs_projectName",
                type: { kind: "base", name: "string" },
                optional: true,
                documentation: "The project name for this reference.",
            },
            {
                name: "_vs_containingType",
                type: { kind: "base", name: "string" },
                optional: true,
                documentation: "The containing type for this reference.",
            },
        ],
        documentation: "A VS-specific reference item with grouping support for Find All References.",
    },
    {
        name: "VSOnAutoInsertParams",
        properties: [
            {
                name: "_vs_textDocument",
                type: { kind: "reference", name: "TextDocumentIdentifier" },
                documentation: "The text document.",
            },
            {
                name: "_vs_position",
                type: { kind: "reference", name: "Position" },
                documentation: "The position inside the text document.",
            },
            {
                name: "_vs_ch",
                type: { kind: "base", name: "string" },
                documentation: "The character that triggered the auto-insert.",
            },
        ],
        documentation: "Parameters for the textDocument/_vs_onAutoInsert request.",
    },
    {
        name: "VSOnAutoInsertResponseItem",
        properties: [
            {
                name: "_vs_textEditFormat",
                type: { kind: "reference", name: "InsertTextFormat" },
                documentation: "The format of the text edit (plaintext or snippet).",
            },
            {
                name: "_vs_textEdit",
                type: { kind: "reference", name: "TextEdit" },
                documentation: "The text edit to apply for the auto-insertion.",
            },
        ],
        documentation: "Response item for the textDocument/_vs_onAutoInsert request.",
    },
    {
        name: "RequestFailureTelemetryEvent",
        properties: [
            {
                name: "eventName",
                type: { kind: "stringLiteral", value: "languageServer.errorResponse" },
                documentation: "The name of the telemetry event.",
            },
            {
                name: "telemetryPurpose",
                type: { kind: "stringLiteral", value: "error" },
                documentation: "Indicates whether the reason for generating the event (e.g. general usage telemetry or errors).",
            },
            {
                name: "properties",
                type: { kind: "reference", name: "RequestFailureTelemetryProperties" },
                documentation: "The properties associated with the event.",
            },
        ],
        documentation: "A RequestFailureTelemetryEvent is sent when a request fails and the server recovers.",
    },
    {
        name: "RequestFailureTelemetryProperties",
        properties: [
            {
                name: "errorCode",
                type: { kind: "base", name: "string" },
                documentation: "The error code associated with the event.",
            },
            {
                name: "requestMethod",
                type: { kind: "base", name: "string" },
                documentation: "The method of the request that caused the event.",
            },
            {
                name: "stack",
                type: { kind: "base", name: "string" },
                documentation: "The stack trace associated with the event.",
            },
        ],
        documentation: "RequestFailureTelemetryProperties contains failure information when an LSP request manages to recover.",
    },
    {
        name: "ProfileParams",
        properties: [
            {
                name: "dir",
                type: { kind: "base", name: "string" },
                documentation: "The directory path where the profile should be saved.",
            },
        ],
        documentation: "Parameters for profiling requests.",
    },
    {
        name: "ProfileResult",
        properties: [
            {
                name: "file",
                type: { kind: "base", name: "string" },
                documentation: "The file path where the profile was saved.",
            },
        ],
        documentation: "Result of a profiling request.",
    },
    {
        name: "InitializeAPISessionParams",
        properties: [
            {
                name: "pipe",
                type: { kind: "base", name: "string" },
                optional: true,
                documentation: "Optional path to use for the named pipe or Unix domain socket. If not provided, a unique path will be generated.",
            },
        ],
        documentation: "Parameters for the initializeAPISession request.",
    },
    {
        name: "InitializeAPISessionResult",
        properties: [
            {
                name: "sessionId",
                type: { kind: "base", name: "string" },
                documentation: "The unique identifier for this API session.",
            },
            {
                name: "pipe",
                type: { kind: "base", name: "string" },
                documentation: "The path to the named pipe or Unix domain socket for API communication.",
            },
        ],
        documentation: "Result for the initializeAPISession request.",
    },
    {
        name: "ProjectInfoParams",
        properties: [
            {
                name: "textDocument",
                type: { kind: "reference", name: "TextDocumentIdentifier" },
                documentation: "The text document to get project info for.",
            },
        ],
        documentation: "Parameters for the custom/projectInfo request.",
    },
    {
        name: "ProjectInfoResult",
        properties: [
            {
                name: "configFilePath",
                type: { kind: "base", name: "string" },
                documentation: "The absolute path to the config file (e.g. /path/to/tsconfig.json) for the project that contains this file, or an empty string if the file is in an inferred project.",
            },
        ],
        documentation: "Result for the custom/projectInfo request.",
    },
    {
        name: "ContentMapperManifest",
        properties: [
            { name: "name", type: { kind: "base", name: "string" }, documentation: "Human-readable mapper name." },
            { name: "version", type: { kind: "base", name: "string" }, optional: true, documentation: "Mapper version." },
            { name: "exec", type: { kind: "array", element: { kind: "base", name: "string" } }, documentation: "Executable and arguments used to start the mapper." },
            { name: "cwd", type: { kind: "base", name: "string" }, optional: true, documentation: "Absolute working directory for the mapper process." },
            { name: "compilerOptions", type: { kind: "array", element: { kind: "base", name: "string" } }, optional: true, documentation: "Compiler option names forwarded to the mapper." },
            { name: "dynamicConfig", type: { kind: "base", name: "boolean" }, optional: true, documentation: "Whether the mapper uses project-scoped dynamic configuration." },
        ],
        documentation: "Inline content mapper manifest supplied by a contributing extension.",
    },
    {
        name: "InferredProjectContentMapperContribution",
        properties: [
            { name: "options", type: { kind: "reference", name: "LSPObject" }, optional: true, documentation: "Options supplied to transforms in inferred projects." },
            { name: "manifest", type: { kind: "reference", name: "ContentMapperManifest" }, documentation: "Inline manifest for the mapper contributed to inferred projects." },
        ],
        documentation: "Content mapper configuration contributed to inferred projects.",
    },
    {
        name: "ContentMapperContribution",
        properties: [
            { name: "contributorId", type: { kind: "base", name: "string" }, documentation: "Unique identifier of the contributor extension." },
            { name: "extensions", type: { kind: "array", element: { kind: "base", name: "string" } }, documentation: "File extensions handled by this content mapper." },
            { name: "inferredProjectContribution", type: { kind: "reference", name: "InferredProjectContentMapperContribution" }, optional: true, documentation: "When present, contributes this mapper to inferred projects." },
        ],
        documentation: "One extension-provided content mapper contribution.",
    },
    {
        name: "SetContentMapperContributionsParams",
        properties: [
            { name: "contributions", type: { kind: "array", element: { kind: "reference", name: "ContentMapperContribution" } }, documentation: "Complete replacement set of active extension contributions." },
            { name: "openDocuments", type: { kind: "array", element: { kind: "reference", name: "TextDocumentIdentifier" } }, documentation: "Currently open documents matching contributed extensions." },
        ],
        documentation: "Parameters for the custom/setContentMapperContributions request.",
    },
    {
        name: "SetLogVerbosityParams",
        properties: [
            {
                name: "verbosity",
                type: { kind: "reference", name: "LogVerbosity" },
                documentation: "The log verbosity level.",
            },
        ],
        documentation: "Parameters for the custom/setLogVerbosity notification.",
    },
    {
        name: "PerformanceStatsTelemetryEvent",
        properties: [
            {
                name: "eventName",
                type: { kind: "stringLiteral", value: "languageServer.performanceStats" },
                documentation: "The name of the telemetry event.",
            },
            {
                name: "telemetryPurpose",
                type: { kind: "stringLiteral", value: "usage" },
                documentation: "Indicates this is a usage telemetry event.",
            },
            {
                name: "measurements",
                type: { kind: "reference", name: "PerformanceStatsTelemetryMeasurements" },
                documentation: "Numeric measurements for this telemetry event.",
            },
        ],
        documentation: "A PerformanceStatsTelemetryEvent is sent periodically with performance and resource usage statistics.",
    },
    {
        name: "PerformanceStatsTelemetryMeasurements",
        properties: [
            { name: "openFileCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Number of files currently open in the editor." },
            { name: "uptimeSeconds", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Seconds since the session was initialized." },
            { name: "projectCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Number of loaded projects." },
            { name: "configCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Number of loaded config files." },
            { name: "cachedDiskFileCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Number of files cached from disk." },
            { name: "memoryUsedBytes", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Total memory mapped by the Go runtime in bytes." },
            { name: "goMemLimit", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "GOMEMLIMIT value in bytes, or 0 if not set." },
            { name: "goGCPercent", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "GOGC percentage value configured for the GC." },
            { name: "heapGoalBytes", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Heap size target the GC is working toward in bytes." },
            { name: "heapLiveBytes", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Bytes of live (reachable) heap objects." },
            { name: "heapObjectCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Number of live or unswept objects occupying heap memory." },
            { name: "heapStackBytes", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Heap memory reserved for goroutine stacks." },
            { name: "heapReleasedBytes", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Heap memory returned to the OS." },
            { name: "heapFreeBytes", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Heap memory that is free and eligible to be returned to the OS." },
            { name: "gcScanHeapBytes", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Total scannable heap bytes — how much the GC must traverse." },
            { name: "goMaxProcs", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "The current GOMAXPROCS value." },
            { name: "goroutineCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Current number of goroutines." },
            { name: "gcCyclesTotal", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Total completed GC cycles." },
            { name: "gcCPUSeconds", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Cumulative CPU time spent in GC in seconds." },
            { name: "userCPUSeconds", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Cumulative CPU time spent in user Go code in seconds." },
            { name: "systemMemTotal", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Total physical memory on the system in bytes." },
            { name: "systemMemUsed", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Used physical memory on the system in bytes." },
            { name: "autoImportProjectBucketCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Number of auto-import project buckets." },
            { name: "autoImportNodeModulesBucketCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Number of auto-import node_modules buckets." },
            { name: "autoImportUniquePackageCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Unique packages across all node_modules buckets." },
            { name: "autoImportProjectExportCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Total indexed exports from project files." },
            { name: "autoImportNodeModulesExportCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Total indexed exports from node_modules." },
            { name: "autoImportProjectFileCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Total files tracked across project buckets." },
            { name: "autoImportNodeModulesFileCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Total files tracked across node_modules buckets." },
            { name: "autoImportNodeModulesUnfilteredBucketCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true, documentation: "Number of node_modules buckets with no package.json filter." },
        ],
        documentation: "Numeric measurements for PerformanceStatsTelemetryEvent.",
    },
    {
        name: "ProjectInfoTelemetryEvent",
        properties: [
            {
                name: "eventName",
                type: { kind: "stringLiteral", value: "languageServer.projectInfo" },
                documentation: "The name of the telemetry event.",
            },
            {
                name: "telemetryPurpose",
                type: { kind: "stringLiteral", value: "usage" },
                documentation: "Indicates this is a usage telemetry event.",
            },
            {
                name: "properties",
                type: { kind: "map", key: { kind: "base", name: "string" }, value: { kind: "base", name: "string" } },
                documentation: "String properties for this telemetry event. Complex values (compilerOptions, fileStats) are JSON-stringified.",
            },
            {
                name: "measurements",
                type: { kind: "reference", name: "ProjectInfoTelemetryMeasurements" },
                documentation: "Numeric measurements for this telemetry event.",
            },
        ],
        documentation: "A ProjectInfoTelemetryEvent is sent once per project when it is first loaded.",
    },
    {
        name: "ProjectInfoTelemetryMeasurements",
        properties: [
            { name: "jsFileCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true },
            { name: "jsFileSize", type: { kind: "base", name: "decimal" }, omitzeroValue: true },
            { name: "jsxFileCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true },
            { name: "jsxFileSize", type: { kind: "base", name: "decimal" }, omitzeroValue: true },
            { name: "tsFileCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true },
            { name: "tsFileSize", type: { kind: "base", name: "decimal" }, omitzeroValue: true },
            { name: "tsxFileCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true },
            { name: "tsxFileSize", type: { kind: "base", name: "decimal" }, omitzeroValue: true },
            { name: "dtsFileCount", type: { kind: "base", name: "decimal" }, omitzeroValue: true },
            { name: "dtsFileSize", type: { kind: "base", name: "decimal" }, omitzeroValue: true },
        ],
        documentation: "Numeric measurements for ProjectInfoTelemetryEvent.",
    },
    {
        name: "MultiDocumentHighlight",
        properties: [
            {
                name: "uri",
                type: { kind: "base", name: "DocumentUri" },
                documentation: "The URI of the document containing the highlights.",
            },
            {
                name: "highlights",
                type: { kind: "array", element: { kind: "reference", name: "DocumentHighlight" } },
                documentation: "The highlights for the document.",
            },
        ],
        documentation: "Represents a collection of document highlights from a single document, used in multi-document highlight responses.",
    },
    {
        name: "MultiDocumentHighlightParams",
        properties: [
            {
                name: "textDocument",
                type: { kind: "reference", name: "TextDocumentIdentifier" },
                documentation: "The text document.",
            },
            {
                name: "position",
                type: { kind: "reference", name: "Position" },
                documentation: "The position inside the text document.",
            },
            {
                name: "filesToSearch",
                type: { kind: "array", element: { kind: "base", name: "DocumentUri" } },
                documentation: "The list of file URIs to search for highlights across.",
            },
        ],
        documentation: "Parameters for the custom/textDocument/multiDocumentHighlight request.",
    },
    {
        name: "VSClassifiedTextRun",
        properties: [
            {
                name: "ClassificationTypeName",
                type: { kind: "base", name: "string" },
                documentation: "The classification type name (e.g. 'keyword', 'class name', 'parameter name').",
            },
            {
                name: "Text",
                type: { kind: "base", name: "string" },
                documentation: "The text content of this run.",
            },
            {
                name: "MarkerTagType",
                type: { kind: "base", name: "string" },
                optional: true,
                documentation: "Optional marker tag type.",
            },
            {
                name: "Style",
                type: { kind: "base", name: "integer" },
                optional: true,
                omitzeroValue: true,
                documentation: "The style of this text run.",
            },
            {
                name: "_vs_type",
                type: { kind: "stringLiteral", value: "ClassifiedTextRun" },
                documentation: "VS type discriminator required by ObjectContentConverter for deserialization.",
            },
        ],
        documentation: "A classified text run with text and classification type, used for colorized display in VS.",
    },
    {
        name: "VSClassifiedTextElement",
        properties: [
            {
                name: "Runs",
                type: { kind: "array", element: { kind: "reference", name: "VSClassifiedTextRun" } },
                documentation: "The classified text runs that make up this element.",
            },
            {
                name: "_vs_type",
                type: { kind: "stringLiteral", value: "ClassifiedTextElement" },
                documentation: "VS type discriminator required by ObjectContentConverter for deserialization.",
            },
        ],
        documentation: "A classified text element containing an array of classified text runs, used for colorized labels in VS.",
    },
    {
        name: "VSImageId",
        properties: [
            {
                name: "Guid",
                type: { kind: "base", name: "string" },
                documentation: "The GUID of the image catalog containing this image.",
            },
            {
                name: "Id",
                type: { kind: "base", name: "integer" },
                documentation: "The numeric identifier of the image within its catalog.",
            },
            {
                name: "_vs_type",
                type: { kind: "stringLiteral", value: "ImageId" },
                documentation: "VS type discriminator required by ObjectContentConverter for deserialization.",
            },
        ],
        documentation: "Identifies an image in a VS image catalog. Used to render symbol-kind icons (e.g. in hover tooltips).",
    },
    {
        name: "VSImageElement",
        properties: [
            {
                name: "ImageId",
                type: { kind: "reference", name: "VSImageId" },
                documentation: "The image to display.",
            },
            {
                name: "_vs_type",
                type: { kind: "stringLiteral", value: "ImageElement" },
                documentation: "VS type discriminator required by ObjectContentConverter for deserialization.",
            },
        ],
        documentation: "An image element (e.g. a symbol-kind icon) for use in VS rich content such as hover tooltips.",
    },
    {
        name: "VSContainerElement",
        properties: [
            {
                name: "Style",
                type: { kind: "reference", name: "VSContainerElementStyle" },
                documentation: "Layout style for the child elements.",
            },
            {
                name: "Elements",
                type: {
                    kind: "array",
                    element: {
                        kind: "or",
                        items: [
                            { kind: "reference", name: "VSImageElement" },
                            { kind: "reference", name: "VSClassifiedTextElement" },
                            { kind: "reference", name: "VSContainerElement" },
                        ],
                    },
                },
                documentation: "The child elements contained within this container.",
            },
            {
                name: "_vs_type",
                type: { kind: "stringLiteral", value: "ContainerElement" },
                documentation: "VS type discriminator required by ObjectContentConverter for deserialization.",
            },
        ],
        documentation: "A container element that groups other VS rich-content elements (images, classified text, or nested containers). Used to build the VS hover raw content that combines a symbol icon with colorized text.",
    },
];

const customEnumerations: Enumeration[] = [
    {
        name: "VSContainerElementStyle",
        type: { kind: "base", name: "integer" },
        values: [
            { name: "Wrapped", value: 0, documentation: "Child elements are laid out inline, wrapping as needed (e.g. an icon next to a signature line)." },
            { name: "Stacked", value: 1, documentation: "Child elements are stacked vertically, each on its own line (e.g. a signature line followed by documentation)." },
        ],
        documentation: "Layout style for a VSContainerElement's children, mirroring VS's Microsoft.VisualStudio.Text.Adornments.ContainerElementStyle.",
    },
    {
        name: "LogVerbosity",
        type: { kind: "base", name: "integer" },
        values: [
            { name: "Off", value: 0, documentation: "All logging disabled." },
            { name: "Trace", value: 1, documentation: "Most verbose; includes LSP request/response traces." },
            { name: "Debug", value: 2, documentation: "Verbose server logs." },
            { name: "Info", value: 3, documentation: "Normal server logs." },
            { name: "Warning", value: 4, documentation: "Warnings only." },
            { name: "Error", value: 5, documentation: "Errors only." },
        ],
        documentation: "Log verbosity level, mirroring the VS Code LogLevel enum values.",
    },
    {
        name: "DiagnosticFlakeLogLevel",
        type: { kind: "base", name: "integer" },
        values: [
            { name: "Off", value: 0, documentation: "All flake logging disabled." },
            { name: "Log", value: 1, documentation: "Log flaky diagnostics to the error log." },
            { name: "Panic", value: 2, documentation: "Panic on flaky diagnostics." },
        ],
        documentation: "Behavior for tracking and logging flaky diagnostics.",
    },
    {
        name: "VSReferenceKind",
        type: { kind: "base", name: "integer" },
        values: [
            { name: "Inactive", value: 0 },
            { name: "Comment", value: 1 },
            { name: "String", value: 2 },
            { name: "Read", value: 3 },
            { name: "Write", value: 4 },
            { name: "Reference", value: 5 },
            { name: "Name", value: 6 },
            { name: "Qualified", value: 7 },
            { name: "TypeArgument", value: 8 },
            { name: "TypeConstraint", value: 9 },
            { name: "BaseType", value: 10 },
            { name: "Constructor", value: 11 },
            { name: "Destructor", value: 12 },
            { name: "Import", value: 13 },
            { name: "Declaration", value: 14 },
            { name: "AddressOf", value: 15 },
            { name: "NotReference", value: 16 },
            { name: "Unknown", value: 17 },
        ],
    },
    {
        name: "CodeLensKind",
        type: {
            kind: "base",
            name: "string",
        },
        values: [
            {
                name: "References",
                value: "references",
            },
            {
                name: "Implementations",
                value: "implementations",
            },
        ],
    },
    {
        name: "AutoImportFixKind",
        type: { kind: "base", name: "integer" },
        values: [
            { name: "UseNamespace", value: 0, documentation: "Augment an existing namespace import." },
            { name: "JsdocTypeImport", value: 1, documentation: "Add a JSDoc-only type import." },
            { name: "AddToExisting", value: 2, documentation: "Insert into an existing import declaration." },
            { name: "AddNew", value: 3, documentation: "Create a fresh import statement." },
            { name: "PromoteTypeOnly", value: 4, documentation: "Promote a type-only import when necessary." },
        ],
    },
    {
        name: "ImportKind",
        type: { kind: "base", name: "integer" },
        values: [
            { name: "Named", value: 0, documentation: "Adds a named import." },
            { name: "Default", value: 1, documentation: "Adds a default import." },
            { name: "Namespace", value: 2, documentation: "Adds a namespace import." },
            { name: "CommonJS", value: 3, documentation: "Adds a CommonJS import assignment." },
        ],
    },
    {
        name: "AddAsTypeOnly",
        type: { kind: "base", name: "integer" },
        values: [
            { name: "Allowed", value: 1, documentation: "Import may be marked type-only if needed." },
            { name: "Required", value: 2, documentation: "Import must be marked type-only." },
            { name: "NotAllowed", value: 4, documentation: "Import cannot be marked type-only." },
        ],
    },
    {
        name: "ClassificationTypeName",
        type: { kind: "base", name: "string" },
        values: [
            { name: "Keyword", value: "keyword", documentation: "Language keyword (e.g., function, const, class)." },
            { name: "Punctuation", value: "punctuation", documentation: "Punctuation characters (e.g., parentheses, commas, semicolons)." },
            { name: "Operator", value: "operator", documentation: "Operators (e.g., =, +, ?)." },
            { name: "WhiteSpace", value: "whitespace", documentation: "Whitespace including spaces and line breaks." },
            { name: "Text", value: "text", documentation: "Plain text with no special classification." },
            { name: "String", value: "string", documentation: "String and literal values." },
            { name: "Number", value: "number", documentation: "Numeric literal values." },
            { name: "Comment", value: "comment", documentation: "Comment text." },
            { name: "ClassName", value: "class name", documentation: "Class names." },
            { name: "InterfaceName", value: "interface name", documentation: "Interface names." },
            { name: "EnumName", value: "enum name", documentation: "Enum names." },
            { name: "ModuleName", value: "module name", documentation: "Module/namespace names." },
            { name: "MethodName", value: "method name", documentation: "Method and function names." },
            { name: "ParameterName", value: "parameter name", documentation: "Parameter names." },
            { name: "PropertyName", value: "property name", documentation: "Property and accessor names." },
            { name: "FieldName", value: "field name", documentation: "Field names (e.g., enum members)." },
            { name: "LocalName", value: "local name", documentation: "Local variable names." },
            { name: "TypeParameterName", value: "type parameter name", documentation: "Type parameter names." },
            { name: "Identifier", value: "identifier", documentation: "General identifiers (e.g., type aliases, imports)." },
        ],
        documentation: "Roslyn classification type names used by VS for syntax coloring in tooltips and other UI elements.",
    },
];
const customRequests: Request[] = [
    {
        method: "custom/runGC",
        typeName: "RunGCRequest",
        messageDirection: "clientToServer",
        result: { kind: "base", name: "null" },
        documentation: "Triggers garbage collection in the language server.",
    },
    {
        method: "custom/saveHeapProfile",
        typeName: "SaveHeapProfileRequest",
        params: { kind: "reference", name: "ProfileParams" },
        messageDirection: "clientToServer",
        result: { kind: "reference", name: "ProfileResult" },
        documentation: "Saves a heap profile to the specified directory.",
    },
    {
        method: "custom/saveAllocProfile",
        typeName: "SaveAllocProfileRequest",
        params: { kind: "reference", name: "ProfileParams" },
        messageDirection: "clientToServer",
        result: { kind: "reference", name: "ProfileResult" },
        documentation: "Saves an allocation profile to the specified directory.",
    },
    {
        method: "custom/startCPUProfile",
        typeName: "StartCPUProfileRequest",
        params: { kind: "reference", name: "ProfileParams" },
        messageDirection: "clientToServer",
        result: { kind: "base", name: "null" },
        documentation: "Starts CPU profiling, writing to the specified directory when stopped.",
    },
    {
        method: "custom/stopCPUProfile",
        typeName: "StopCPUProfileRequest",
        messageDirection: "clientToServer",
        result: { kind: "reference", name: "ProfileResult" },
        documentation: "Stops CPU profiling and saves the profile.",
    },
    {
        method: "custom/initializeAPISession",
        typeName: "CustomInitializeAPISessionRequest",
        params: { kind: "reference", name: "InitializeAPISessionParams" },
        result: { kind: "reference", name: "InitializeAPISessionResult" },
        messageDirection: "clientToServer",
        documentation: "Custom request to initialize an API session.",
    },
    {
        method: "custom/projectInfo",
        typeName: "CustomProjectInfoRequest",
        params: { kind: "reference", name: "ProjectInfoParams" },
        result: { kind: "reference", name: "ProjectInfoResult" },
        messageDirection: "clientToServer",
        documentation: "Returns project information (e.g. the tsconfig.json path) for a given text document.",
    },
    {
        method: "custom/setContentMapperContributions",
        typeName: "CustomSetContentMapperContributionsRequest",
        params: { kind: "reference", name: "SetContentMapperContributionsParams" },
        result: { kind: "base", name: "null" },
        messageDirection: "clientToServer",
        documentation: "Replaces extension content mapper contributions and discovers configured mappers for matching open documents.",
    },
    {
        method: "custom/textDocument/sourceDefinition",
        typeName: "CustomTextDocumentSourceDefinitionRequest",
        params: { kind: "reference", name: "TextDocumentPositionParams" },
        result: { kind: "reference", name: "LocationOrLocationsOrDefinitionLinksOrNull" },
        messageDirection: "clientToServer",
        documentation: "Request to get source definitions for a position.",
    },
    {
        method: "custom/textDocument/multiDocumentHighlight",
        typeName: "CustomMultiDocumentHighlightRequest",
        params: { kind: "reference", name: "MultiDocumentHighlightParams" },
        result: {
            kind: "or",
            items: [
                { kind: "array", element: { kind: "reference", name: "MultiDocumentHighlight" } },
                { kind: "base", name: "null" },
            ],
        },
        messageDirection: "clientToServer",
        documentation: "Request to get document highlights across multiple files.",
    },
    {
        method: "textDocument/_vs_onAutoInsert",
        typeName: "VSOnAutoInsertRequest",
        params: { kind: "reference", name: "VSOnAutoInsertParams" },
        result: {
            kind: "or",
            items: [
                { kind: "reference", name: "VSOnAutoInsertResponseItem" },
                { kind: "base", name: "null" },
            ],
        },
        messageDirection: "clientToServer",
        documentation: "Request for auto-insert when a trigger character is typed (VS-specific).",
    },
    {
        method: "textDocument/_vs_references",
        typeName: "VSReferencesRequest",
        params: { kind: "reference", name: "ReferenceParams" },
        result: {
            kind: "or",
            items: [
                { kind: "array", element: { kind: "reference", name: "VSReferenceItem" } },
                { kind: "base", name: "null" },
            ],
        },
        messageDirection: "clientToServer",
        documentation: "VS-specific request for Find All References with grouped reference items.",
    },
];

const customNotifications: Notification[] = [
    {
        method: "custom/setLogVerbosity",
        typeName: "CustomSetLogVerbosityNotification",
        params: { kind: "reference", name: "SetLogVerbosityParams" },
        messageDirection: "clientToServer",
        documentation: "Notification to set the server's log verbosity level based on the output channel's log level.",
    },
];

// compareStructures is the set of generated structures for which a Compare method should be emitted.
// The Compare method defines a total ordering by comparing fields in declaration order.
// All listed structures (and any structure-typed fields they reference) must contain only
// comparable fields: base scalar types, or other structures that are themselves in this set.
const compareStructures = new Set<string>([
    "Position",
    "Range",
    "TextEdit",
]);

const customTypeAliases: TypeAlias[] = [
    {
        name: "TelemetryEvent",
        type: {
            kind: "or",
            items: [
                { kind: "reference", name: "RequestFailureTelemetryEvent" },
                { kind: "reference", name: "PerformanceStatsTelemetryEvent" },
                { kind: "reference", name: "ProjectInfoTelemetryEvent" },
                { kind: "base", name: "null" },
            ],
        },
    },
];

// Track which custom Data structures were declared explicitly
const explicitDataStructures = new Set(customStructures.map(s => s.name));

// Map from registration method → { fieldName, optionsTypeName }
// Built during patchAndPreprocessModel, used during code generation.
interface RegistrationMethodInfo {
    registrationMethod: string;
    fieldName: string;
    optionsTypeName: string;
    isRegistrationOnly?: boolean;
}
let registrationMethods: RegistrationMethodInfo[] = [];

// Patch and preprocess the model
function patchAndPreprocessModel() {
    // Track which Data types we need to create as placeholders
    const neededDataStructures = new Set<string>();

    // Collect all registration option types from requests and notifications
    const registrationOptionTypes: Type[] = [];
    for (const request of [...model.requests, ...model.notifications]) {
        if (request.registrationOptions) {
            registrationOptionTypes.push(request.registrationOptions);
        }
    }

    // Create synthetic structures for "and" types in registration options
    const syntheticStructures: Structure[] = [];
    for (let i = 0; i < registrationOptionTypes.length; i++) {
        const regOptType = registrationOptionTypes[i];
        if (regOptType.kind === "and") {
            // Find which request/notification this registration option belongs to
            const owner = [...model.requests, ...model.notifications].find(r => r.registrationOptions === regOptType);
            if (!owner) {
                throw new Error("Could not find owner for 'and' type registration option");
            }

            // Determine the proper name based on the typeName or method
            let structureName: string;
            if (owner.typeName) {
                // Use typeName as base: "ColorPresentationRequest" -> "ColorPresentationRegistrationOptions"
                structureName = owner.typeName.replace(/Request$/, "").replace(/Notification$/, "") + "RegistrationOptions";
            }
            else {
                // Fall back to method: "textDocument/colorPresentation" -> "ColorPresentationRegistrationOptions"
                const methodParts = owner.method.split("/");
                const lastPart = methodParts[methodParts.length - 1];
                structureName = titleCase(lastPart) + "RegistrationOptions";
            }

            // Extract all reference types from the "and"
            const refTypes = regOptType.items.filter((item): item is ReferenceType => item.kind === "reference");

            // Create a synthetic structure that combines all the referenced structures
            syntheticStructures.push({
                name: structureName,
                properties: [],
                extends: refTypes,
                documentation: `Registration options for ${owner.method}.`,
            });

            // Replace the "and" type with a reference to the synthetic structure
            registrationOptionTypes[i] = { kind: "reference", name: structureName };
            // Also update the model so the request/notification has the resolved type
            owner.registrationOptions = registrationOptionTypes[i];
        }
    }

    for (const structure of model.structures) {
        // Patch ServerCapabilities to add custom tsgo capability flags
        if (structure.name === "ServerCapabilities") {
            structure.properties.push({
                name: "_vs_onAutoInsertProvider",
                type: { kind: "reference", name: "VSOnAutoInsertOptions" },
                optional: true,
                documentation: "Provider options for the VS auto-insert feature via textDocument/_vs_onAutoInsert.",
            });
            structure.properties.push({
                name: "_vs_referencesProvider",
                type: { kind: "base", name: "boolean" },
                optional: true,
                documentation: "The server provides VS-specific grouped references via textDocument/_vs_references.",
            });
        }

        // Patch HoverParams to add verbosityLevel
        if (structure.name === "HoverParams") {
            structure.properties.push({
                name: "verbosityLevel",
                type: { kind: "base", name: "integer" },
                optional: true,
                documentation: "Controls how many levels of type definitions will be expanded. Default is 0.",
            });
        }

        // Patch WorkspaceSymbolParams to optionally scope the search to projects
        // containing a document, matching Strada's currentProject mode.
        if (structure.name === "WorkspaceSymbolParams") {
            structure.properties.push({
                name: "textDocument",
                type: { kind: "reference", name: "TextDocumentIdentifier" },
                optional: true,
                documentation: "Scopes the workspace symbol search to projects containing this document.",
            });
        }

        // Patch Hover to add canIncreaseVerbosity
        if (structure.name === "Hover") {
            structure.properties.push(
                {
                    name: "canIncreaseVerbosity",
                    type: { kind: "base", name: "boolean" },
                    omitzeroValue: true,
                    documentation: "Whether the verbosity level can be increased for this hover.",
                },
                {
                    name: "_vs_rawContent",
                    type: { kind: "reference", name: "VSContainerElement" },
                    optional: true,
                    documentation: "VS-specific rich content (symbol icon + colorized/classified text) rendered by clients that support Visual Studio extensions, in place of `contents`.",
                },
            );
        }

        // Patch ClientCapabilities to add VS-specific client capabilities
        if (structure.name === "ClientCapabilities") {
            structure.properties.push(
                {
                    name: "_vs_supportsVisualStudioExtensions",
                    type: { kind: "base", name: "boolean" },
                    optional: true,
                    documentation: "Whether the client supports Visual Studio extensions.",
                },
                {
                    name: "_vs_supportedSnippetVersion",
                    type: { kind: "base", name: "integer" },
                    optional: true,
                    documentation: "The snippet version supported by the client.",
                },
                {
                    name: "_vs_supportsNotIncludingTextInTextDocumentDidOpen",
                    type: { kind: "base", name: "boolean" },
                    optional: true,
                    documentation: "Whether the client supports not including text in textDocument/didOpen notifications.",
                },
                {
                    name: "_vs_supportsIconExtensions",
                    type: { kind: "base", name: "boolean" },
                    optional: true,
                    documentation: "Whether the client supports icon extensions.",
                },
                {
                    name: "_vs_supportsDiagnosticRequests",
                    type: { kind: "base", name: "boolean" },
                    optional: true,
                    documentation: "Whether the client supports diagnostic requests.",
                },
            );
        }

        // Patch SignatureInformation to add VS-specific colorized label
        if (structure.name === "SignatureInformation") {
            structure.properties.push({
                name: "_vs_colorizedLabel",
                type: { kind: "reference", name: "VSClassifiedTextElement" },
                optional: true,
                documentation: "A colorized label for the signature, providing classified text runs for VS syntax coloring.",
            });
        }

        for (const prop of structure.properties) {
            // Replace initializationOptions type with custom InitializationOptions.
            // The spec types this field as LSPAny?, which includes null, so keep
            // it nullable so a null value sent by loose clients is accepted.
            if (prop.name === "initializationOptions" && prop.type.kind === "reference" && prop.type.name === "LSPAny") {
                prop.type = {
                    kind: "or",
                    items: [
                        { kind: "reference", name: "InitializationOptions" },
                        { kind: "base", name: "null" },
                    ],
                };
            }

            // Replace Data *any fields with custom typed Data fields
            if (prop.name === "data" && prop.type.kind === "reference" && prop.type.name === "LSPAny") {
                const customDataType = `${structure.name}Data`;
                prop.type = { kind: "reference", name: customDataType };

                // If we haven't explicitly declared this Data structure, we'll need a placeholder
                if (!explicitDataStructures.has(customDataType)) {
                    neededDataStructures.add(customDataType);
                }
            }

            // Registration.registerOptions and Registration.method are handled specially:
            // registerOptions becomes a custom struct, and method is derived from it.
            // Remove both from the structure so the normal generator skips them.
            if (structure.name === "Registration" && (prop.name === "registerOptions" || prop.name === "method")) {
                // Will be filtered out below
            }

            // Replace ProgressParams.value with a proper union type
            if (structure.name === "ProgressParams" && prop.name === "value" && prop.type.kind === "reference" && prop.type.name === "LSPAny") {
                prop.type = {
                    kind: "or",
                    items: [
                        { kind: "reference", name: "WorkDoneProgressBegin" },
                        { kind: "reference", name: "WorkDoneProgressReport" },
                        { kind: "reference", name: "WorkDoneProgressEnd" },
                    ],
                };
            }
        }
    }

    for (const notification of model.notifications) {
        if (notification.typeName === "TelemetryEventNotification") {
            notification.params = {
                kind: "reference",
                name: "TelemetryEvent",
            };
        }
    }

    // Create placeholder structures for Data types that weren't explicitly declared
    for (const dataTypeName of neededDataStructures) {
        const baseName = dataTypeName.replace(/Data$/, "");
        customStructures.push({
            name: dataTypeName,
            properties: [],
            documentation: `${dataTypeName} is a placeholder for custom data preserved on a ${baseName}.`,
        });
    }

    // Add custom enumerations, custom structures, custom requests, and synthetic structures to the model
    model.enumerations.push(...customEnumerations);
    model.structures.push(...customStructures, ...syntheticStructures);
    model.requests.push(...customRequests);
    model.notifications.push(...customNotifications);

    // Build structure map for preprocessing
    const structureMap = new Map<string, Structure>();
    for (const structure of model.structures) {
        structureMap.set(structure.name, structure);
    }

    function collectInheritedProperties(structure: Structure, visited = new Set<string>()): Property[] {
        if (visited.has(structure.name)) {
            return []; // Avoid circular dependencies
        }
        visited.add(structure.name);

        const properties: Property[] = [];
        const inheritanceTypes = [...(structure.extends || []), ...(structure.mixins || [])];

        for (const type of inheritanceTypes) {
            if (type.kind === "reference") {
                const inheritedStructure = structureMap.get(type.name);
                if (inheritedStructure) {
                    properties.push(
                        ...collectInheritedProperties(inheritedStructure, new Set(visited)),
                        ...inheritedStructure.properties,
                    );
                }
            }
        }

        return properties;
    }

    // Inline inheritance for each structure
    for (const structure of model.structures) {
        const inheritedProperties = collectInheritedProperties(structure);

        // Merge properties with structure's own properties taking precedence
        const propertyMap = new Map<string, Property>();

        inheritedProperties.forEach(prop => propertyMap.set(prop.name, prop));
        structure.properties.forEach(prop => propertyMap.set(prop.name, prop));

        structure.properties = Array.from(propertyMap.values());
        structure.extends = undefined;
        structure.mixins = undefined;

        // Replace experimental LSPAny with typed ExperimentalClientCapabilities in ClientCapabilities
        if (structure.name === "ClientCapabilities") {
            const expProp = structure.properties.find(p => p.name === "experimental");
            if (expProp) {
                expProp.type = { kind: "reference", name: "ExperimentalClientCapabilities" };
                expProp.optional = true;
            }
        }

        // Replace experimental LSPAny with typed ExperimentalServerCapabilities in ServerCapabilities
        if (structure.name === "ServerCapabilities") {
            const expProp = structure.properties.find(p => p.name === "experimental");
            if (expProp) {
                expProp.type = { kind: "reference", name: "ExperimentalServerCapabilities" };
                expProp.optional = true;
            }
        }

        // Remove method and registerOptions from Registration (handled by custom codegen)
        if (structure.name === "Registration") {
            structure.properties = structure.properties.filter(p => p.name !== "method" && p.name !== "registerOptions");
        }
    }

    // Remove _InitializeParams structure after flattening (it was only needed for inheritance)
    model.structures = model.structures.filter(s => s.name !== "_InitializeParams");

    // Remove all notebook-related features from the model
    function isNotebookRelatedName(name: string): boolean {
        const lower = name.toLowerCase();
        return lower.includes("notebook");
    }

    function isNotebookRelatedMethod(method: string): boolean {
        return method.toLowerCase().startsWith("notebookdocument/");
    }

    function typeReferencesNotebook(type: Type): boolean {
        if (type.kind === "reference") return isNotebookRelatedName(type.name);
        if (type.kind === "array") return typeReferencesNotebook(type.element);
        if (type.kind === "or" || type.kind === "and") return type.items.some(typeReferencesNotebook);
        if (type.kind === "map") return typeReferencesNotebook(type.key) || typeReferencesNotebook(type.value);
        return false;
    }

    function isEntirelyNotebookType(type: Type): boolean {
        if (type.kind === "reference") return isNotebookRelatedName(type.name);
        if (type.kind === "array") return isEntirelyNotebookType(type.element);
        if (type.kind === "or" || type.kind === "and") return type.items.every(isEntirelyNotebookType);
        return false;
    }

    function removeNotebookFromType(type: Type): Type {
        if (type.kind === "or") {
            const filtered = type.items.filter(item => !typeReferencesNotebook(item)).map(removeNotebookFromType);
            if (filtered.length === 1) return filtered[0];
            if (filtered.length < type.items.length) {
                return { ...type, items: filtered };
            }
        }
        if (type.kind === "and") {
            const filtered = type.items.filter(item => !typeReferencesNotebook(item)).map(removeNotebookFromType);
            if (filtered.length === 1) return filtered[0];
            if (filtered.length < type.items.length) {
                return { ...type, items: filtered };
            }
        }
        return type;
    }

    // Filter out notebook structures (and notebook-only structures like ExecutionSummary)
    const notebookOnlyStructures = new Set(["ExecutionSummary"]);
    model.structures = model.structures.filter(s => !isNotebookRelatedName(s.name) && !notebookOnlyStructures.has(s.name));

    // Remove notebook properties from remaining structures
    for (const structure of model.structures) {
        structure.properties = structure.properties.filter(p => {
            if (isNotebookRelatedName(p.name)) return false;
            // Only remove properties whose type is entirely notebook-related
            if (isEntirelyNotebookType(p.type)) return false;
            return true;
        });
        // Clean up union types in remaining properties to remove notebook members
        for (const prop of structure.properties) {
            prop.type = removeNotebookFromType(prop.type);
        }
    }

    // Filter out notebook notifications and requests
    model.notifications = model.notifications.filter(n => !isNotebookRelatedMethod(n.method));
    model.requests = model.requests.filter(r => !isNotebookRelatedMethod(r.method));

    // Filter out notebook enumerations
    model.enumerations = model.enumerations.filter(e => !isNotebookRelatedName(e.name));

    // Remove notebook-related values from remaining enumerations
    for (const enumeration of model.enumerations) {
        enumeration.values = enumeration.values.filter(v => !isNotebookRelatedName(v.name));
    }

    // Filter out notebook type aliases
    model.typeAliases = model.typeAliases.filter(ta => !isNotebookRelatedName(ta.name));

    // Clean up type aliases that reference notebook types (e.g., DocumentFilter)
    for (const ta of model.typeAliases) {
        if (ta.type.kind === "or") {
            ta.type.items = ta.type.items.filter(item => !typeReferencesNotebook(item));
            // If only one item remains, unwrap the union
            if (ta.type.items.length === 1) {
                ta.type = ta.type.items[0];
            }
        }
    }

    // Build the registration method map (after notebook filtering).
    // Each unique registration method gets a field in the generated RegisterOptions struct.
    const regMethodSeen = new Set<string>();
    for (const request of [...model.requests, ...model.notifications]) {
        if (!request.registrationOptions) continue;
        const regMethod = (request as any).registrationMethod || request.method;

        if (regMethodSeen.has(regMethod)) continue;
        regMethodSeen.add(regMethod);

        // Resolve the options type name
        const ro = request.registrationOptions;
        let optionsTypeName: string;
        if (ro.kind === "reference") {
            optionsTypeName = ro.name;
        }
        else {
            throw new Error(`Unexpected registrationOptions kind '${ro.kind}' for ${request.method}; expected all to be resolved to references`);
        }

        registrationMethods.push({
            registrationMethod: regMethod,
            fieldName: methodNameIdentifier(regMethod),
            optionsTypeName,
        });
    }

    // Identify registration-only methods (not also a request/notification method).
    // These need their own Method constant emitted.
    const allRequestMethods = new Set([...model.requests, ...model.notifications].map(r => r.method));
    for (const reg of registrationMethods) {
        (reg as any).isRegistrationOnly = !allRequestMethods.has(reg.registrationMethod);
    }

    // Merge LSPErrorCodes into ErrorCodes and remove LSPErrorCodes
    const errorCodesEnum = model.enumerations.find(e => e.name === "ErrorCodes");
    const lspErrorCodesEnum = model.enumerations.find(e => e.name === "LSPErrorCodes");
    if (errorCodesEnum && lspErrorCodesEnum) {
        // Merge LSPErrorCodes values into ErrorCodes
        errorCodesEnum.values.push(...lspErrorCodesEnum.values);
        // Remove LSPErrorCodes from the model
        model.enumerations = model.enumerations.filter(e => e.name !== "LSPErrorCodes");
    }

    // Singularize plural enum names (e.g., "ErrorCodes" -> "ErrorCode")
    for (const enumeration of model.enumerations) {
        if (enumeration.name.endsWith("Codes")) {
            enumeration.name = enumeration.name.slice(0, -1); // "Codes" -> "Code"
        }
        else if (enumeration.name.endsWith("Modifiers")) {
            enumeration.name = enumeration.name.slice(0, -1); // "Modifiers" -> "Modifier"
        }
        else if (enumeration.name.endsWith("Types")) {
            enumeration.name = enumeration.name.slice(0, -1); // "Types" -> "Type"
        }
    }
}

patchAndPreprocessModel();

// Validate that telemetry events in the TelemetryEvent union have properly shaped
// measurements and properties fields. measurements struct fields must only contain
// numeric types (decimal/integer/uinteger).
function validateTelemetryEvents() {
    const telemetryAlias = customTypeAliases.find(a => a.name === "TelemetryEvent");
    if (!telemetryAlias || telemetryAlias.type.kind !== "or") return;

    const structureMap = new Map(model.structures.map(s => [s.name, s]));

    for (const item of telemetryAlias.type.items) {
        if (item.kind !== "reference") continue;
        const eventStruct = structureMap.get(item.name);
        if (!eventStruct) continue;

        for (const prop of eventStruct.properties) {
            if (prop.name === "measurements" && prop.type.kind === "reference") {
                const measurementsStruct = structureMap.get(prop.type.name);
                if (!measurementsStruct) continue;
                for (const mp of measurementsStruct.properties) {
                    if (mp.type.kind !== "base" || !["decimal", "integer", "uinteger"].includes(mp.type.name)) {
                        throw new Error(
                            `Telemetry measurements struct ${prop.type.name}.${mp.name} must be a numeric type ` +
                                `(decimal/integer/uinteger), got ${mp.type.kind === "base" ? mp.type.name : mp.type.kind}`,
                        );
                    }
                }
            }
        }
    }
}

validateTelemetryEvents();

interface GoType {
    name: string;
    needsPointer: boolean;
}

interface TypeInfo {
    types: Map<string, GoType>;
    literalTypes: Map<string, string>;
    unionTypes: Map<string, { name: string; type: Type; containedNull: boolean; }[]>;
    typeAliasMap: Map<string, Type>;
}

const typeInfo: TypeInfo = {
    types: new Map(),
    literalTypes: new Map(),
    unionTypes: new Map(),
    typeAliasMap: new Map(),
};

function titleCase(s: string) {
    return s.charAt(0).toUpperCase() + s.slice(1);
}

function goFieldName(prop: Property): string {
    if (prop.name.startsWith("_vs_")) {
        return "VS" + titleCase(prop.name.slice(4));
    }
    return titleCase(prop.name);
}

function resolveType(type: Type): GoType {
    switch (type.kind) {
        case "base":
            switch (type.name) {
                case "integer":
                    return { name: "int32", needsPointer: false };
                case "uinteger":
                    return { name: "uint32", needsPointer: false };
                case "string":
                    return { name: "string", needsPointer: false };
                case "boolean":
                    return { name: "bool", needsPointer: false };
                case "URI":
                    return { name: "URI", needsPointer: false };
                case "DocumentUri":
                    return { name: "DocumentUri", needsPointer: false };
                case "decimal":
                    return { name: "float64", needsPointer: false };
                case "null":
                    return { name: "any", needsPointer: false };
                default:
                    throw new Error(`Unsupported base type: ${type.name}`);
            }

        case "reference":
            const typeAliasOverride = typeAliasOverrides.get(type.name);
            if (typeAliasOverride) {
                return typeAliasOverride;
            }

            const nonResolved = nonResolvedAliases.has(type.name);
            if (nonResolved) {
                return { name: type.name, needsPointer: false };
            }

            // Check if this is a type alias that resolves to a union type
            const aliasedType = typeInfo.typeAliasMap.get(type.name);
            if (aliasedType) {
                return resolveType(aliasedType);
            }

            let refType = typeInfo.types.get(type.name);
            if (!refType) {
                refType = { name: type.name, needsPointer: true };
                typeInfo.types.set(type.name, refType);
            }
            return refType;

        case "array": {
            const elementType = resolveType(type.element);
            const arrayTypeName = elementType.needsPointer
                ? `[]*${elementType.name}`
                : `[]${elementType.name}`;
            return {
                name: arrayTypeName,
                needsPointer: false,
            };
        }

        case "map": {
            const keyType = resolveType(type.key);
            const valueType = resolveType(type.value);
            const valueTypeName = valueType.needsPointer ? `*${valueType.name}` : valueType.name;

            return {
                name: `map[${keyType.name}]${valueTypeName}`,
                needsPointer: false,
            };
        }

        case "tuple": {
            if (
                type.items.length === 2 &&
                type.items[0].kind === "base" && type.items[0].name === "uinteger" &&
                type.items[1].kind === "base" && type.items[1].name === "uinteger"
            ) {
                return { name: "[2]uint32", needsPointer: false };
            }

            throw new Error("Unsupported tuple type: " + JSON.stringify(type));
        }

        case "stringLiteral": {
            const typeName = `StringLiteral${type.value.split(".").map(titleCase).join("")}`;
            typeInfo.literalTypes.set(String(type.value), typeName);
            return { name: typeName, needsPointer: false };
        }

        case "integerLiteral": {
            const typeName = `IntegerLiteral${type.value}`;
            typeInfo.literalTypes.set(String(type.value), typeName);
            return { name: typeName, needsPointer: false };
        }

        case "booleanLiteral": {
            const typeName = `BooleanLiteral${type.value ? "True" : "False"}`;
            typeInfo.literalTypes.set(String(type.value), typeName);
            return { name: typeName, needsPointer: false };
        }
        case "literal":
            if (type.value.properties.length === 0) {
                return { name: "struct{}", needsPointer: false };
            }

            throw new Error("Unexpected non-empty literal object: " + JSON.stringify(type.value));

        case "or": {
            return handleOrType(type);
        }

        default:
            throw new Error(`Unsupported type kind: ${type.kind}`);
    }
}

function flattenOrTypes(types: Type[]): Type[] {
    const flattened = new Set<Type>();

    for (const rawType of types) {
        let type = rawType;

        // Dereference reference types that point to OR types
        if (rawType.kind === "reference") {
            const aliasedType = typeInfo.typeAliasMap.get(rawType.name);
            if (aliasedType && aliasedType.kind === "or") {
                type = aliasedType;
            }
        }

        if (type.kind === "or") {
            // Recursively flatten OR types
            for (const subType of flattenOrTypes(type.items)) {
                flattened.add(subType);
            }
        }
        else {
            flattened.add(rawType);
        }
    }

    return Array.from(flattened);
}

function pluralize(name: string): string {
    // Handle common irregular plurals and special cases
    if (
        name.endsWith("s") || name.endsWith("x") || name.endsWith("z") ||
        name.endsWith("ch") || name.endsWith("sh")
    ) {
        return name + "es";
    }
    if (name.endsWith("y") && name.length > 1 && !"aeiou".includes(name[name.length - 2])) {
        return name.slice(0, -1) + "ies";
    }
    return name + "s";
}

function handleOrType(orType: OrType): GoType {
    // First, flatten any nested OR types
    const types = flattenOrTypes(orType.items);

    // Check for nullable types (OR with null)
    const nullIndex = types.findIndex(item => item.kind === "base" && item.name === "null");
    let containedNull = nullIndex !== -1;

    // If it's nullable, remove the null type from the list
    let nonNullTypes = types;
    if (containedNull) {
        nonNullTypes = types.filter((_, i) => i !== nullIndex);
    }

    // If no types remain after filtering null, this shouldn't happen
    if (nonNullTypes.length === 0) {
        throw new Error("Union type with only null is not supported: " + JSON.stringify(types));
    }

    // Even if only one type remains after filtering null, we still need to create a union type
    // to preserve the nullable behavior (all fields nil = null)

    let memberNames = nonNullTypes.map(type => {
        if (type.kind === "reference") {
            return type.name;
        }
        else if (type.kind === "base") {
            return titleCase(type.name);
        }
        else if (
            type.kind === "array" &&
            (type.element.kind === "reference" || type.element.kind === "base")
        ) {
            return pluralize(titleCase(type.element.name));
        }
        else if (type.kind === "array") {
            // Handle more complex array types
            const elementType = resolveType(type.element);
            return `${elementType.name}Array`;
        }
        else if (type.kind === "literal" && type.value.properties.length === 0) {
            return "EmptyObject";
        }
        else if (type.kind === "tuple") {
            return "Tuple";
        }
        else {
            throw new Error(`Unsupported type kind in union: ${type.kind}`);
        }
    });

    // Find longest common prefix of member names chunked by PascalCase
    function findLongestCommonPrefix(names: string[]): string {
        if (names.length === 0) return "";
        if (names.length === 1) return "";

        // Split each name into PascalCase chunks
        function splitPascalCase(name: string): string[] {
            const chunks: string[] = [];
            let currentChunk = "";

            for (let i = 0; i < name.length; i++) {
                const char = name[i];
                if (char >= "A" && char <= "Z" && currentChunk.length > 0) {
                    // Start of a new chunk
                    chunks.push(currentChunk);
                    currentChunk = char;
                }
                else {
                    currentChunk += char;
                }
            }

            if (currentChunk.length > 0) {
                chunks.push(currentChunk);
            }

            return chunks;
        }

        const allChunks = names.map(splitPascalCase);
        const minChunkLength = Math.min(...allChunks.map(chunks => chunks.length));

        // Find the longest common prefix of chunks
        let commonChunks: string[] = [];
        for (let i = 0; i < minChunkLength; i++) {
            const chunk = allChunks[0][i];
            if (allChunks.every(chunks => chunks[i] === chunk)) {
                commonChunks.push(chunk);
            }
            else {
                break;
            }
        }

        return commonChunks.join("");
    }

    const commonPrefix = findLongestCommonPrefix(memberNames);

    let unionTypeName = "";

    if (commonPrefix.length > 0) {
        const trimmedMemberNames = memberNames.map(name => name.slice(commonPrefix.length));
        if (trimmedMemberNames.every(name => name)) {
            unionTypeName = commonPrefix + trimmedMemberNames.join("Or");
            memberNames = trimmedMemberNames;
        }
        else {
            unionTypeName = memberNames.join("Or");
        }
    }
    else {
        unionTypeName = memberNames.join("Or");
    }

    if (containedNull) {
        unionTypeName += "OrNull";
    }
    else {
        containedNull = false;
    }

    const union = memberNames.map((name, i) => ({ name, type: nonNullTypes[i], containedNull }));

    typeInfo.unionTypes.set(unionTypeName, union);

    return {
        name: unionTypeName,
        needsPointer: false,
    };
}

const typeAliasOverrides = new Map([
    ["LSPAny", { name: "any", needsPointer: false }],
    ["LSPArray", { name: "[]any", needsPointer: false }],
    ["LSPObject", { name: "map[string]any", needsPointer: false }],
    ["uint64", { name: "uint64", needsPointer: false }],
]);

// These type aliases are intentionally not resolved to their underlying types.
// It means that we can end up with non-normalized union types in some places.
// Also, unlike other type aliases, these will get a type alias in the generated source code.
// We may want to eventually do this for all type aliases though.
const nonResolvedAliases = new Set(customTypeAliases.map(ta => ta.name));

/**
 * First pass: Resolve all type information
 */
function collectTypeDefinitions() {
    // Process all enumerations first to make them available for struct fields
    for (const enumeration of model.enumerations) {
        typeInfo.types.set(enumeration.name, {
            name: enumeration.name,
            needsPointer: false,
        });
    }

    const valueTypes = new Set([
        "Position",
        "Range",
        "Location",
        "Color",
        "TextDocumentIdentifier",
        "PreviousResultId",
        "VersionedTextDocumentIdentifier",
        "OptionalVersionedTextDocumentIdentifier",
        "ExportInfoMapKey",
    ]);

    // Process all structures
    for (const structure of model.structures) {
        typeInfo.types.set(structure.name, {
            name: structure.name,
            needsPointer: !valueTypes.has(structure.name),
        });
    }

    // Process all type aliases
    for (const typeAlias of model.typeAliases) {
        if (typeAliasOverrides.has(typeAlias.name)) {
            continue;
        }

        // Store the alias mapping so we can resolve it later
        typeInfo.typeAliasMap.set(typeAlias.name, typeAlias.type);
    }
}

function formatDocumentation(s: string | undefined): string {
    if (!s) return "";

    let lines: string[] = [];

    for (let line of s.split("\n")) {
        line = line.trimEnd();
        line = line.replace(/(\w ) +/g, "$1");
        // Some upstream docs include dangling block comment delimiters; remove them
        // so they don't leak into generated `//` comments.
        line = line.replace(/\s*\/\*+\s*/g, " ");
        line = line.replace(/\s*\*+\/\s*/g, " ");
        line = line.replace(/\s{2,}/g, " ").trimEnd();
        line = line.replace(/\{@link(?:code)?.*?([^} ]+)\}/g, "$1");
        line = line.replace(/^@(since|proposed|deprecated)(.*)/, (_, tag, rest) => {
            lines.push("");
            return `${titleCase(tag)}${rest ? ":" + rest : "."}`;
        });
        lines.push(line);
    }

    // filter out contiguous empty lines
    while (true) {
        const toRemove = lines.findIndex((line, index) => {
            if (line) return false;
            if (index === 0) return true;
            if (index === lines.length - 1) return true;
            return !(lines[index - 1] && lines[index + 1]);
        });
        if (toRemove === -1) break;
        lines.splice(toRemove, 1);
    }

    return lines.length > 0 ? "// " + lines.join("\n// ") + "\n" : "";
}

function methodNameIdentifier(name: string) {
    return name.split("/").map(v => {
        if (v === "$") return "";
        // Mirror goFieldName: "_vs_foo" -> "VSFoo".
        if (v.startsWith("_vs_")) return "VS" + titleCase(v.slice(4));
        return titleCase(v);
    }).join("");
}

/**
 * Returns the JSON token kind ("string", "number", "object", "array", "boolean")
 * for a given meta model Type, or undefined if the kind cannot be statically determined.
 */
function jsonKindForType(type: Type): string | undefined {
    switch (type.kind) {
        case "base":
            switch (type.name) {
                case "integer":
                case "uinteger":
                case "decimal":
                    return "number";
                case "string":
                case "URI":
                case "DocumentUri":
                    return "string";
                case "boolean":
                    return "boolean";
                default:
                    return undefined;
            }
        case "reference": {
            if (typeAliasOverrides.has(type.name)) {
                return undefined;
            }
            if (model.structures.some(s => s.name === type.name)) {
                return "object";
            }
            const enumeration = model.enumerations.find(e => e.name === type.name);
            if (enumeration) {
                switch (enumeration.type.name) {
                    case "string":
                        return "string";
                    case "integer":
                    case "uinteger":
                        return "number";
                    default:
                        return undefined;
                }
            }
            const aliasType = typeInfo.typeAliasMap.get(type.name);
            if (aliasType) return jsonKindForType(aliasType);
            return undefined;
        }
        case "array":
            return "array";
        case "map":
            return "object";
        case "tuple":
            return "array";
        case "stringLiteral":
            return "string";
        case "integerLiteral":
            return "number";
        case "booleanLiteral":
            return "boolean";
        case "literal":
            return "object";
        case "or": {
            const kinds = new Set(type.items.map(item => jsonKindForType(item)).filter(Boolean));
            return kinds.size === 1 ? kinds.values().next().value : undefined;
        }
        default:
            return undefined;
    }
}

function goKindCasesForJsonKind(kind: string): string {
    switch (kind) {
        case "string":
            return `case '"':`;
        case "number":
            return `case '0':`;
        case "object":
            return `case '{':`;
        case "array":
            return `case '[':`;
        case "boolean":
            return `case 't', 'f':`;
        default:
            return "";
    }
}

/**
 * Checks if a meta model Type can represent a JSON null value.
 * Used to determine whether to reject explicit JSON `null` for any field
 * that can otherwise decode `null` without a type error.
 */
function typeCanBeNull(type: Type): boolean {
    switch (type.kind) {
        case "base":
            return type.name === "null";
        case "reference": {
            const override = typeAliasOverrides.get(type.name);
            if (override) {
                return override.name === "any";
            }
            // A bare "any" reference resolves to Go's `any` (interface), which can hold null.
            if (type.name === "any") {
                return true;
            }
            if (nonResolvedAliases.has(type.name)) {
                const customAlias = customTypeAliases.find(t => t.name === type.name);
                if (customAlias) return typeCanBeNull(customAlias.type);
                return false;
            }
            const aliased = typeInfo.typeAliasMap.get(type.name);
            if (aliased) return typeCanBeNull(aliased);
            return false;
        }
        case "or":
            return type.items.some(item => typeCanBeNull(item));
        default:
            return false;
    }
}

/**
 * For a group of union entries that share the same JSON kind (e.g., all objects),
 * find a discriminator field — a JSON property whose string literal type differs
 * across variants — enabling efficient O(1) dispatch instead of try-each.
 */
function findDiscriminatorField(entries: { fieldName: string; typeName: string; originalType: Type; }[]): {
    fieldName: string;
    mapping: Map<string, { fieldName: string; typeName: string; originalType: Type; }>;
    unmapped: { fieldName: string; typeName: string; originalType: Type; }[];
} | null {
    // For each entry, find string literal fields and build candidate discriminators.
    // A valid discriminator is a field name that appears on multiple variants with
    // different string literal values.
    const fieldCandidates = new Map<string, Map<string, typeof entries[0] | undefined>>();

    for (const entry of entries) {
        if (entry.originalType.kind !== "reference") continue;
        const structure = model.structures.find(s => s.name === (entry.originalType as ReferenceType).name);
        if (!structure) continue;

        for (const prop of structure.properties) {
            if (prop.type.kind === "stringLiteral") {
                if (!fieldCandidates.has(prop.name)) {
                    fieldCandidates.set(prop.name, new Map());
                }

                const mapping = fieldCandidates.get(prop.name)!;
                if (!mapping.has(prop.type.value)) {
                    mapping.set(prop.type.value, entry);
                }
                else {
                    // Two entries share the same literal value; invalidate this candidate.
                    mapping.set(prop.type.value, undefined);
                }
            }
        }
    }

    // Pick the discriminator field that covers the most entries.
    let bestField: string | null = null;
    let bestMapping: Map<string, typeof entries[0]> | null = null;

    for (const [fieldName, mapping] of fieldCandidates) {
        const validMapping = new Map<string, typeof entries[0]>();
        for (const [value, entry] of mapping) {
            if (entry !== undefined) validMapping.set(value, entry);
        }
        if (validMapping.size >= 2 && (!bestMapping || validMapping.size > bestMapping.size)) {
            bestField = fieldName;
            bestMapping = validMapping;
        }
    }

    if (!bestField || !bestMapping) return null;

    const mappedEntries = new Set(bestMapping.values());
    const unmapped = entries.filter(e => !mappedEntries.has(e));

    return { fieldName: bestField, mapping: bestMapping, unmapped };
}

function hasCustomStructureCodec(name: string): boolean {
    return name === "Registration";
}

/**
 * For a group of union entries that share the same JSON kind, find fields whose
 * presence/absence in the JSON uniquely identifies a variant. A "presence discriminator"
 * for variant X is a required field on X that does not appear in any other variant's
 * property set at all.
 */
function findPresenceDiscriminator(entries: { fieldName: string; typeName: string; originalType: Type; }[]): {
    checks: { jsonFieldName: string; entry: { fieldName: string; typeName: string; originalType: Type; }; }[];
    unmapped: { fieldName: string; typeName: string; originalType: Type; }[];
} | null {
    // Collect all property names for each variant
    const variantProps = new Map<typeof entries[0], { required: Property[]; allNames: Set<string>; }>();
    for (const entry of entries) {
        if (entry.originalType.kind !== "reference") continue;
        const structure = model.structures.find(s => s.name === (entry.originalType as ReferenceType).name);
        if (!structure) continue;
        const required = structure.properties.filter(p => !p.optional && !p.omitzeroValue);
        const allNames = new Set(structure.properties.map(p => p.name));
        variantProps.set(entry, { required, allNames });
    }

    const checks: { jsonFieldName: string; entry: typeof entries[0]; }[] = [];
    const handled = new Set<typeof entries[0]>();

    for (const entry of entries) {
        const info = variantProps.get(entry);
        if (!info) continue;

        const otherEntries = entries.filter(e => e !== entry);
        for (const field of info.required) {
            const absentFromAllOthers = otherEntries.every(other => {
                const otherInfo = variantProps.get(other);
                if (!otherInfo) return false;
                return !otherInfo.allNames.has(field.name);
            });
            if (absentFromAllOthers) {
                checks.push({ jsonFieldName: field.name, entry });
                handled.add(entry);
                break;
            }
        }
    }

    if (checks.length === 0) return null;

    const unmapped = entries.filter(e => !handled.has(e));
    return { checks, unmapped };
}
// ---------------------------------------------------------------------------------------------------------
// Rust emitter. Everything above this line is Go's model processing; everything below replaces Go's
// generateCode().
// ---------------------------------------------------------------------------------------------------------

const RUST_KEYWORDS = new Set([
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
    "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box",
    "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen",
]);

// Same snake_case rule as the rest of the port (acronym runs are one word).
function snake(s: string): string {
    return s.replace(/([a-z0-9])([A-Z])/g, "$1_$2").replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2").toLowerCase();
}

function rustIdent(goName: string): string {
    const n = snake(goName);
    return RUST_KEYWORDS.has(n) ? n + "_" : n;
}

function screaming(goName: string): string {
    return snake(goName).toUpperCase();
}

function rustStr(s: string): string {
    return `"${s.replace(/\\/g, "\\\\").replace(/"/g, "\\\"")}"`;
}

// Rust type model. `named` covers structures, unions, enumerations and literal types.
type RType =
    | { k: "prim"; name: string; copy: boolean; hash: boolean; }
    | { k: "named"; name: string; }
    | { k: "vec"; el: RType; }
    | { k: "map"; key: RType; val: RType; }
    | { k: "opt"; el: RType; }
    | { k: "box"; el: RType; };

const PRIMS: Record<string, { name: string; copy: boolean; hash: boolean; }> = {
    "int32": { name: "i32", copy: true, hash: true },
    "uint32": { name: "u32", copy: true, hash: true },
    "uint64": { name: "u64", copy: true, hash: true },
    "float64": { name: "f64", copy: true, hash: false },
    "bool": { name: "bool", copy: true, hash: true },
    "string": { name: "String", copy: false, hash: true },
    "any": { name: "Value", copy: false, hash: false },
    "[2]uint32": { name: "[u32; 2]", copy: true, hash: true },
    "struct{}": { name: "EmptyObject", copy: true, hash: true },
    "URI": { name: "URI", copy: false, hash: true },
    "DocumentUri": { name: "DocumentUri", copy: false, hash: true },
};

// Maps a Go type as resolveType names it ("[]*TextEdit", "map[DocumentUri][]*TextEdit", "int32", ...).
function rustOfGo(g: string): RType {
    if (g.startsWith("*")) {
        throw new Error(`unexpected pointer type ${g}`);
    }
    if (g.startsWith("[]")) {
        let el = g.slice(2);
        if (el.startsWith("*")) el = el.slice(1);
        return { k: "vec", el: rustOfGo(el) };
    }
    if (g.startsWith("map[")) {
        // A pointer map value can be nil (JSON null): Option.
        const close = g.indexOf("]");
        const val = g.slice(close + 1);
        const key = rustOfGo(g.slice(4, close));
        if (val.startsWith("*")) return { k: "map", key, val: { k: "opt", el: rustOfGo(val.slice(1)) } };
        return { k: "map", key, val: rustOfGo(val) };
    }
    const prim = PRIMS[g];
    if (prim) return { k: "prim", ...prim };
    return { k: "named", name: g };
}

function renderType(t: RType): string {
    switch (t.k) {
        case "prim":
        case "named":
            return t.name;
        case "vec":
            return `Vec<${renderType(t.el)}>`;
        case "map":
            return `OrderedMap<${renderType(t.key)}, ${renderType(t.val)}>`;
        case "opt":
            return `Option<${renderType(t.el)}>`;
        case "box":
            return `Box<${renderType(t.el)}>`;
    }
}

interface RField {
    prop?: Property;
    goField: string; // Go field name
    rust: string; // Rust field name
    goType: string; // Go field type, with the pointer if any
    type: RType; // Rust field type
}

interface UnionArm {
    fieldName: string; // Go field name
    typeName: string; // Go member type (without the pointer)
    originalType: Type;
}

function generateCode() {
    const parts: string[] = [];

    function write(s: string) {
        parts.push(s);
    }

    function writeLine(s = "") {
        parts.push(s + "\n");
    }

    function writeDoc(doc: string | undefined, indent = "") {
        const formatted = formatDocumentation(doc);
        for (const line of formatted.split("\n").filter(l => l)) {
            writeLine(indent + line);
        }
    }

    const requestsAndNotifications: (Request | Notification)[] = [...model.requests, ...model.notifications];

    // Resolve every type in the order Go's emitter does, so that union and literal types are registered (and
    // later emitted) in Go's order.
    for (const structure of model.structures) {
        for (const prop of structure.properties) resolveType(prop.type);
    }
    for (const request of requestsAndNotifications) {
        if ("result" in request && !(request.result.kind === "base" && request.result.name === "null")) {
            resolveType(request.result);
        }
        if (request.params && !Array.isArray(request.params)) resolveType(request.params);
    }
    for (const alias of customTypeAliases) resolveType(alias.type);
    for (const [, members] of typeInfo.unionTypes) {
        for (const member of members) resolveType(member.type);
    }

    const structureMap = new Map(model.structures.map(s => [s.name, s]));
    const enumerationMap = new Map(model.enumerations.map(e => [e.name, e]));

    // Union arms, deduplicated by Go type exactly like Go's union emitter.
    const unionArms = new Map<string, UnionArm[]>();
    for (const [name, members] of typeInfo.unionTypes) {
        const uniqueTypeFields = new Map<string, string>();
        const uniqueTypeToOriginal = new Map<string, Type>();
        for (const member of members) {
            const memberType = resolveType(member.type).name;
            if (!uniqueTypeFields.has(memberType)) {
                uniqueTypeFields.set(memberType, titleCase(member.name));
                uniqueTypeToOriginal.set(memberType, member.type);
            }
        }
        unionArms.set(
            name,
            Array.from(uniqueTypeFields.entries()).map(([typeName, fieldName]) => ({
                fieldName,
                typeName,
                originalType: uniqueTypeToOriginal.get(typeName)!,
            })),
        );
    }

    // Unboxed field types, used to find recursion.
    function structFieldTypesUnboxed(structure: Structure): RType[] {
        return structure.properties.map(prop => {
            const type = resolveType(prop.type);
            const base = rustOfGo(type.name);
            return prop.optional && !prop.omitzeroValue ? { k: "opt", el: base } as RType : base;
        });
    }

    function unionArmTypesUnboxed(name: string): RType[] {
        return unionArms.get(name)!.map(arm => ({ k: "opt", el: rustOfGo(arm.typeName) }) as RType);
    }

    function namedFieldTypes(name: string): RType[] {
        const structure = structureMap.get(name);
        if (structure) return structFieldTypesUnboxed(structure);
        if (unionArms.has(name)) return unionArmTypesUnboxed(name);
        return [];
    }

    // Named types stored inline (not behind a Vec or map).
    function inlineNames(t: RType): string[] {
        switch (t.k) {
            case "named":
                return [t.name];
            case "opt":
            case "box":
                return inlineNames(t.el);
            default:
                return [];
        }
    }

    function reachesInline(from: string, target: string): boolean {
        const visited = new Set<string>();
        const stack = [from];
        while (stack.length > 0) {
            const name = stack.pop()!;
            if (name === target) return true;
            if (visited.has(name)) continue;
            visited.add(name);
            for (const t of namedFieldTypes(name)) stack.push(...inlineNames(t));
        }
        return false;
    }

    // A field whose type contains its container inline gets a Box.
    function boxIfRecursive(container: string, t: RType): RType {
        switch (t.k) {
            case "named":
                return reachesInline(t.name, container) ? { k: "box", el: t } : t;
            case "opt":
                return { k: "opt", el: boxIfRecursive(container, t.el) };
            default:
                return t;
        }
    }

    const structFields = new Map<string, RField[]>();
    for (const structure of model.structures) {
        structFields.set(
            structure.name,
            structure.properties.map(prop => {
                const type = resolveType(prop.type);
                const goType = (prop.optional || type.needsPointer) && !prop.omitzeroValue ? `*${type.name}` : type.name;
                let rt = boxIfRecursive(structure.name, rustOfGo(type.name));
                if (prop.optional && !prop.omitzeroValue) rt = { k: "opt", el: rt };
                return { prop, goField: goFieldName(prop), rust: rustIdent(goFieldName(prop)), goType, type: rt };
            }),
        );
    }

    const unionFields = new Map<string, RField[]>();
    for (const [name, arms] of unionArms) {
        unionFields.set(
            name,
            arms.map(arm => ({
                goField: arm.fieldName,
                rust: rustIdent(arm.fieldName),
                goType: `*${arm.typeName}`,
                type: { k: "opt", el: boxIfRecursive(name, rustOfGo(arm.typeName)) } as RType,
            })),
        );
    }

    function fieldsOf(name: string): RField[] | undefined {
        return structFields.get(name) ?? unionFields.get(name);
    }

    // Derivable traits: Copy when every field is Copy; Eq + Hash when every field is (no float, any or map).
    function traitHolds(t: RType, trait: "copy" | "hash", visiting: Set<string>): boolean {
        switch (t.k) {
            case "prim":
                return t[trait];
            case "named": {
                if (enumerationMap.has(t.name) || typeInfo.literalTypes.has(t.name) || [...typeInfo.literalTypes.values()].includes(t.name)) {
                    return true;
                }
                if (hasCustomStructureCodec(t.name)) return false;
                const fields = fieldsOf(t.name);
                if (!fields) return true;
                if (visiting.has(t.name)) return true;
                visiting.add(t.name);
                const result = fields.every(f => traitHolds(f.type, trait, visiting));
                visiting.delete(t.name);
                return result;
            }
            case "vec":
                return trait === "hash" && traitHolds(t.el, trait, visiting);
            case "map":
                return false;
            case "opt":
                return traitHolds(t.el, trait, visiting);
            case "box":
                return trait === "hash" && traitHolds(t.el, trait, visiting);
        }
    }

    function derives(name: string): string {
        const fields = fieldsOf(name)!;
        const extra: string[] = [];
        if (hasCustomStructureCodec(name)) {
            // Registration also holds RegisterOptions.
            return `#[derive(Clone, Debug, Default, PartialEq)]`;
        }
        if (fields.every(f => traitHolds(f.type, "copy", new Set([name])))) extra.push("Copy");
        if (fields.every(f => traitHolds(f.type, "hash", new Set([name])))) extra.push("Eq", "Hash");
        return `#[derive(Clone, Debug, Default, PartialEq${extra.map(e => ", " + e).join("")})]`;
    }

    // Go's struct codec strictness (structcodec.go): required fields, and nilable fields that reject null.
    function rejectsNull(field: RField): boolean {
        const prop = field.prop!;
        const type = resolveType(prop.type);
        const nilable = prop.optional || type.needsPointer || type.name.startsWith("[]") || type.name.startsWith("map[") || !!prop.omitzeroValue;
        const nullable = nilable && (typeCanBeNull(prop.type) || !!prop.omitzeroValue);
        const goNilable = field.goType.startsWith("*") || field.goType.startsWith("[]") || field.goType.startsWith("map[");
        return goNilable && !nullable;
    }

    function isStrictStructure(structure: Structure): boolean {
        const requiredProps = structure.properties.filter(p => !p.optional && !p.omitzeroValue);
        const hasNullRejectableFields = structure.properties.some(p => {
            if (p.omitzeroValue) return false;
            if (typeCanBeNull(p.type)) return false;
            const resolved = resolveType(p.type);
            return p.optional || resolved.needsPointer || resolved.name.startsWith("[]") || resolved.name.startsWith("map[");
        });
        return (requiredProps.length > 0 || hasNullRejectableFields) && !hasCustomStructureCodec(structure.name);
    }

    // File header
    writeLine("// Code generated by tools/gen-lsproto/generate.mts; DO NOT EDIT.");
    writeLine("");
    writeLine("use std::fmt;");
    writeLine("");
    writeLine("use tsrs_core::collections::OrderedMap;");
    writeLine("");
    writeLine("use crate::json::{is_null, kind, IsZero, Json, JsonError, ObjectWriter, Value};");
    writeLine("use crate::lsp::*;");
    writeLine("use crate::structcodec::*;");
    writeLine("");
    writeLine("// Meta model version " + model.metaData.version);
    writeLine("");

    // Generate structures
    writeLine("// Structures");
    writeLine("");

    function emitStructJson(structure: Structure, fields: RField[]) {
        const strict = isStrictStructure(structure);
        writeLine(`impl Json for ${structure.name} {`);
        writeLine(`    const GO_TYPE: &'static str = "lsproto.${structure.name}";`);
        if (typeInfo.types.get(structure.name)!.needsPointer) {
            writeLine(`    const GO_POINTER: bool = true;`);
        }
        writeLine("");
        writeLine("    fn to_json(&self) -> Value {");
        writeLine(`        let ${fields.length === 0 ? "" : "mut "}w = ObjectWriter::new(${fields.length});`);
        for (const f of fields) {
            const prop = f.prop!;
            const useOmitzero = prop.optional || prop.omitzeroValue;
            let method = "field";
            if (useOmitzero) {
                if (f.goType.startsWith("*")) {
                    method = "opt";
                }
                else {
                    if (f.goType.startsWith("[]") || f.goType.startsWith("map[") || resolveType(prop.type).needsPointer) {
                        throw new Error(`unsupported omitzero value field ${structure.name}.${prop.name}`);
                    }
                    method = "value";
                }
            }
            writeLine(`        w.${method}(${rustStr(prop.name)}, &self.${f.rust});`);
        }
        writeLine("        w.finish()");
        writeLine("    }");
        writeLine("");
        writeLine("    fn from_json(v: &Value) -> Result<Self, JsonError> {");
        if (fields.length === 0) {
            writeLine(`        struct_members(v, Self::GO_TYPE, ${strict})?;`);
            writeLine("        Ok(Self::default())");
        }
        else {
            const required = fields.filter(f => strict && !f.prop!.optional && !f.prop!.omitzeroValue);
            writeLine("        let mut s = Self::default();");
            writeLine(`        let Some(members) = struct_members(v, Self::GO_TYPE, ${strict})? else {`);
            writeLine("            return Ok(s);");
            writeLine("        };");
            if (required.length > 0) {
                writeLine("        let mut seen = 0u64;");
            }
            writeLine("        for (k, v) in members {");
            writeLine("            match k.as_str() {");
            for (const f of fields) {
                const prop = f.prop!;
                const isOpt = f.type.k === "opt";
                const bit = required.indexOf(f);
                let expr: string;
                if (!strict) {
                    expr = isOpt ? "opt(k, v)?" : "field(k, v)?";
                }
                else if (bit >= 0) {
                    expr = rejectsNull(f) ? "field_non_null(k, v, Self::GO_TYPE)?" : "field(k, v)?";
                }
                else if (isOpt) {
                    expr = rejectsNull(f) ? "opt_non_null(k, v, Self::GO_TYPE)?" : "opt(k, v)?";
                }
                else {
                    expr = "field(k, v)?";
                }
                if (bit >= 0) {
                    writeLine(`                ${rustStr(prop.name)} => {`);
                    writeLine(`                    seen |= 1 << ${bit};`);
                    writeLine(`                    s.${f.rust} = ${expr};`);
                    writeLine("                }");
                }
                else {
                    writeLine(`                ${rustStr(prop.name)} => s.${f.rust} = ${expr},`);
                }
            }
            writeLine("                _ => {}");
            writeLine("            }");
            writeLine("        }");
            if (required.length > 0) {
                writeLine(`        check_required(seen, &[${required.map(f => rustStr(f.prop!.name)).join(", ")}], Self::GO_TYPE)?;`);
            }
            writeLine("        Ok(s)");
        }
        writeLine("    }");
        writeLine("}");
        writeLine("");
    }

    function emitRegistration(structure: Structure, fields: RField[]) {
        // RegisterOptions struct
        writeLine(`// RegisterOptions is an externally-tagged union representing the options for a capability registration.`);
        writeLine(`// Exactly one field should be set. The set field determines the method for the registration.`);
        writeLine(`#[derive(Clone, Debug, Default, PartialEq)]`);
        writeLine(`pub struct RegisterOptions {`);
        for (const reg of registrationMethods) {
            writeLine(`    pub ${rustIdent(reg.fieldName)}: Option<${reg.optionsTypeName}>,`);
        }
        writeLine(`}`);
        writeLine("");

        writeLine(`impl RegisterOptions {`);
        writeLine(`    fn count_non_nil(&self) -> usize {`);
        writeLine(`        let mut count = 0;`);
        for (const reg of registrationMethods) {
            writeLine(`        count += self.${rustIdent(reg.fieldName)}.is_some() as usize;`);
        }
        writeLine(`        count`);
        writeLine(`    }`);
        writeLine(`}`);
        writeLine("");

        const idField = fields.find(f => f.prop!.name === "id")!;
        writeLine(`impl Json for Registration {`);
        writeLine(`    const GO_TYPE: &'static str = "lsproto.Registration";`);
        writeLine(`    const GO_POINTER: bool = true;`);
        writeLine("");
        writeLine(`    fn to_json(&self) -> Value {`);
        writeLine(`        let Some(register_options) = &self.register_options else {`);
        writeLine(`            panic!("RegisterOptions must be set");`);
        writeLine(`        };`);
        writeLine(`        assert_only_one("exactly one element of RegisterOptions should be set", register_options.count_non_nil());`);
        writeLine("");
        writeLine(`        let mut w = ObjectWriter::new(3);`);
        writeLine(`        w.field("id", &self.${idField.rust});`);
        writeLine(`        let (method, opts) = if false {`);
        writeLine(`            unreachable!()`);
        for (const reg of registrationMethods) {
            writeLine(`        } else if let Some(o) = &register_options.${rustIdent(reg.fieldName)} {`);
            writeLine(`            (${rustStr(reg.registrationMethod)}, o.to_json())`);
        }
        writeLine(`        } else {`);
        writeLine(`            unreachable!()`);
        writeLine(`        };`);
        writeLine(`        w.raw("method", Value::String(method.to_string()));`);
        writeLine(`        w.raw("registerOptions", opts);`);
        writeLine(`        w.finish()`);
        writeLine(`    }`);
        writeLine("");
        writeLine(`    fn from_json(v: &Value) -> Result<Self, JsonError> {`);
        writeLine(`        let mut s = Registration::default();`);
        writeLine(`        const MISSING_ID: u32 = 1 << 0;`);
        writeLine(`        const MISSING_METHOD: u32 = 1 << 1;`);
        writeLine(`        let mut missing = MISSING_ID | MISSING_METHOD;`);
        writeLine("");
        writeLine(`        let Some(members) = struct_members(v, Self::GO_TYPE, true)? else {`);
        writeLine(`            unreachable!()`);
        writeLine(`        };`);
        writeLine("");
        writeLine(`        let mut method = String::new();`);
        writeLine(`        let mut raw_register_options: Option<&Value> = None;`);
        writeLine("");
        writeLine(`        for (k, v) in members {`);
        writeLine(`            match k.as_str() {`);
        writeLine(`                "id" => {`);
        writeLine(`                    missing &= !MISSING_ID;`);
        writeLine(`                    s.${idField.rust} = field(k, v)?;`);
        writeLine(`                }`);
        writeLine(`                "method" => {`);
        writeLine(`                    missing &= !MISSING_METHOD;`);
        writeLine(`                    method = field(k, v)?;`);
        writeLine(`                }`);
        writeLine(`                "registerOptions" => raw_register_options = Some(v),`);
        writeLine(`                _ => {}`);
        writeLine(`            }`);
        writeLine(`        }`);
        writeLine("");
        writeLine(`        if missing != 0 {`);
        writeLine(`            let mut missing_props = Vec::new();`);
        writeLine(`            if missing & MISSING_ID != 0 {`);
        writeLine(`                missing_props.push("id");`);
        writeLine(`            }`);
        writeLine(`            if missing & MISSING_METHOD != 0 {`);
        writeLine(`                missing_props.push("method");`);
        writeLine(`            }`);
        writeLine(`            return Err(JsonError::method(Self::GO_TYPE, err_missing(&missing_props)));`);
        writeLine(`        }`);
        writeLine("");
        writeLine(`        if let Some(raw_register_options) = raw_register_options {`);
        writeLine(`            let mut register_options = RegisterOptions::default();`);
        writeLine(`            match Method::from_str(&method) {`);
        for (const reg of registrationMethods) {
            writeLine(`                Method::${reg.fieldName} => {`);
            writeLine(`                    register_options.${rustIdent(reg.fieldName)} = arm_buffered(raw_register_options)?;`);
            writeLine(`                }`);
        }
        writeLine(`                _ => {`);
        writeLine(`                    return Err(JsonError::method(Self::GO_TYPE, format!("unknown registration method: {method}")));`);
        writeLine(`                }`);
        writeLine(`            }`);
        writeLine(`            s.register_options = Some(register_options);`);
        writeLine(`        } else {`);
        writeLine(`            return Err(JsonError::method(Self::GO_TYPE, format!("missing registerOptions for method: {method}")));`);
        writeLine(`        }`);
        writeLine("");
        writeLine(`        Ok(s)`);
        writeLine(`    }`);
        writeLine(`}`);
        writeLine("");
    }

    function generateCompareMethod(structure: Structure, fields: RField[]) {
        writeLine(`impl ${structure.name} {`);
        writeLine(`    pub fn compare(&self, other: &${structure.name}) -> i32 {`);
        for (let i = 0; i < fields.length; i++) {
            const f = fields[i];
            const isLast = i === fields.length - 1;
            const expr = compareExpressionForProperty(structure.name, f);
            if (isLast) {
                writeLine(`        ${expr}`);
            }
            else {
                writeLine(`        let c = ${expr};`);
                writeLine(`        if c != 0 {`);
                writeLine(`            return c;`);
                writeLine(`        }`);
            }
        }
        writeLine(`    }`);
        writeLine(`}`);
        writeLine("");
    }

    function compareExpressionForProperty(structName: string, f: RField): string {
        const prop = f.prop!;
        if (prop.type.kind === "reference") {
            const refName = prop.type.name;
            if (compareStructures.has(refName)) {
                if (f.type.k !== "named") {
                    throw new Error(`Cannot generate Compare for ${structName}.${f.goField}: pointer field`);
                }
                return `self.${f.rust}.compare(&other.${f.rust})`;
            }
        }

        if (prop.type.kind === "base") {
            switch (prop.type.name) {
                case "string":
                case "URI":
                case "DocumentUri":
                case "integer":
                case "uinteger":
                case "decimal":
                    return `cmp_compare(&self.${f.rust}, &other.${f.rust})`;
            }
        }

        throw new Error(`Cannot generate Compare for ${structName}.${f.goField}: unsupported field type ${JSON.stringify(prop.type)}. Add support in compareExpressionForProperty.`);
    }

    for (const structure of model.structures) {
        const fields = structFields.get(structure.name)!;

        write(formatDocumentation(structure.documentation));
        writeLine(derives(structure.name));
        writeLine(`pub struct ${structure.name} {`);
        for (let i = 0; i < fields.length; i++) {
            const f = fields[i];
            if (i > 0) writeLine("");
            writeDoc(f.prop!.documentation, "    ");
            writeLine(`    pub ${f.rust}: ${renderType(f.type)},`);
        }
        // Special: add RegisterOptions field to Registration
        if (structure.name === "Registration") {
            writeLine("");
            writeLine(`    // Options necessary for the registration. Determines the method.`);
            writeLine(`    pub register_options: Option<RegisterOptions>,`);
        }
        writeLine("}");
        writeLine("");

        if (hasTextDocumentURI(structure)) {
            const textDocProp = structure.properties?.find(p => (p.name === "textDocument" || p.name === "_vs_textDocument") && p.type.kind === "reference" && p.type.name === "TextDocumentIdentifier");
            const textDocFieldName = textDocProp ? goFieldName(textDocProp) : "TextDocument";
            writeLine(`impl HasTextDocumentURI for ${structure.name} {`);
            writeLine(`    fn text_document_uri(&self) -> &DocumentUri {`);
            writeLine(`        &self.${rustIdent(textDocFieldName)}.uri`);
            writeLine(`    }`);
            writeLine(`}`);
            writeLine("");

            if (hasTextDocumentPosition(structure)) {
                const posProp = structure.properties?.find(p => (p.name === "position" || p.name === "_vs_position") && p.type.kind === "reference" && p.type.name === "Position");
                const posFieldName = posProp ? goFieldName(posProp) : "Position";
                writeLine(`impl HasTextDocumentPosition for ${structure.name} {`);
                writeLine(`    fn text_document_position(&self) -> Position {`);
                writeLine(`        self.${rustIdent(posFieldName)}`);
                writeLine(`    }`);
                writeLine(`}`);
                writeLine("");
            }
        }

        const locationUriProperty = getLocationUriProperty(structure);
        if (locationUriProperty) {
            writeLine(`impl HasLocation for ${structure.name} {`);
            writeLine(`    fn get_location(&self) -> Location {`);
            if (locationUriProperty === "Uri" && structure.name === "Location") {
                writeLine(`        self.clone()`);
            }
            else {
                writeLine(`        Location {`);
                writeLine(`            uri: self.${rustIdent(locationUriProperty)}.clone(),`);
                writeLine(`            range: self.${rustIdent(locationUriProperty.replace(/Uri$/, "Range"))},`);
                writeLine(`        }`);
            }
            writeLine(`    }`);
            writeLine(`}`);
            writeLine("");
        }

        if (structure.name === "Registration") {
            emitRegistration(structure, fields);
        }
        else {
            emitStructJson(structure, fields);
        }

        if (compareStructures.has(structure.name)) {
            generateCompareMethod(structure, fields);
        }
    }

    // Helper function to detect if an enum is a bitflag enum
    // Hardcoded list of bitflag enums
    const bitflagEnums = new Set(["WatchKind"]);

    function isBitflagEnum(enumeration: any): boolean {
        return bitflagEnums.has(enumeration.name);
    }

    // Generate enumerations
    writeLine("// Enumerations");
    writeLine("");

    for (const enumeration of model.enumerations) {
        let baseType;
        switch (enumeration.type.name) {
            case "string":
                baseType = "&'static str";
                break;
            case "integer":
                baseType = "i32";
                break;
            case "uinteger":
                baseType = "u32";
                break;
            default:
                throw new Error(`Unsupported enum type: ${enumeration.type.name}`);
        }

        write(formatDocumentation(enumeration.documentation));
        writeLine(`#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]`);
        writeLine(`pub struct ${enumeration.name}(pub ${baseType});`);
        writeLine("");

        const enumValues = enumeration.values.map(value => ({
            value: String(value.value),
            numericValue: Number(value.value),
            name: value.name,
            identifier: titleCase(value.name),
            documentation: value.documentation,
            deprecated: value.deprecated,
        }));

        writeLine(`impl ${enumeration.name} {`);
        for (const entry of enumValues) {
            writeDoc(entry.documentation, "    ");
            let valueLiteral;
            if (enumeration.type.name === "string") {
                valueLiteral = rustStr(entry.value.replace(/^"|"$/g, ""));
            }
            else {
                valueLiteral = entry.value;
            }
            writeLine(`    pub const ${entry.identifier}: ${enumeration.name} = ${enumeration.name}(${valueLiteral});`);
        }
        writeLine(`}`);
        writeLine("");

        // Go's String() method for non-string enums
        if (enumeration.type.name !== "string") {
            const sortedValues = [...enumValues].sort((a, b) => a.numericValue - b.numericValue);
            writeLine(`impl ${enumeration.name} {`);
            writeLine(`    pub fn string(&self) -> String {`);
            if (isBitflagEnum(enumeration)) {
                writeLine(`        if self.0 == 0 {`);
                writeLine(`            return "0".to_string();`);
                writeLine(`        }`);
                writeLine(`        let mut parts: Vec<&str> = Vec::new();`);
                for (const v of sortedValues) {
                    writeLine(`        if self.0 & ${v.numericValue} != 0 {`);
                    writeLine(`            parts.push(${rustStr(v.name)});`);
                    writeLine(`        }`);
                }
                writeLine(`        if parts.is_empty() {`);
                writeLine(`            return format!("${enumeration.name}({})", self.0);`);
                writeLine(`        }`);
                writeLine(`        parts.join("|")`);
            }
            else {
                // The first name of a value in sorted order wins, like Go's stringer-style switch.
                const seen = new Set<number>();
                writeLine(`        match self.0 {`);
                for (const v of sortedValues) {
                    if (seen.has(v.numericValue)) continue;
                    seen.add(v.numericValue);
                    writeLine(`            ${v.numericValue} => ${rustStr(v.name)}.to_string(),`);
                }
                writeLine(`            _ => format!("${enumeration.name}({})", self.0),`);
                writeLine(`        }`);
            }
            writeLine(`    }`);
            writeLine(`}`);
            writeLine("");

            writeLine(`impl fmt::Display for ${enumeration.name} {`);
            writeLine(`    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {`);
            writeLine(`        f.write_str(&self.string())`);
            writeLine(`    }`);
            writeLine(`}`);
            writeLine("");
        }

        // Go's Error() method for ErrorCode
        if (enumeration.name === "ErrorCode") {
            writeLine(`impl std::error::Error for ${enumeration.name} {}`);
            writeLine("");
        }

        writeLine(`impl Json for ${enumeration.name} {`);
        writeLine(`    const GO_TYPE: &'static str = "lsproto.${enumeration.name}";`);
        writeLine("");
        writeLine(`    fn to_json(&self) -> Value {`);
        if (enumeration.type.name === "string") {
            writeLine(`        Value::String(self.0.to_string())`);
        }
        else {
            writeLine(`        Value::Number(self.0 as f64)`);
        }
        writeLine(`    }`);
        writeLine("");
        writeLine(`    fn from_json(v: &Value) -> Result<Self, JsonError> {`);
        if (enumeration.type.name === "string") {
            writeLine(`        crate::json::decode_string(v, Self::GO_TYPE).map(|s| ${enumeration.name}(intern(s)))`);
        }
        else if (enumeration.type.name === "integer") {
            writeLine(`        crate::json::decode_int(v, 32, Self::GO_TYPE).map(|n| ${enumeration.name}(n as i32))`);
        }
        else {
            writeLine(`        crate::json::decode_uint(v, 32, Self::GO_TYPE).map(|n| ${enumeration.name}(n as u32))`);
        }
        writeLine(`    }`);
        writeLine(`}`);
        writeLine("");

        writeLine(`impl IsZero for ${enumeration.name} {`);
        writeLine(`    fn is_zero(&self) -> bool {`);
        if (enumeration.type.name === "string") {
            writeLine(`        self.0.is_empty()`);
        }
        else {
            writeLine(`        self.0 == 0`);
        }
        writeLine(`    }`);
        writeLine(`}`);
        writeLine("");
    }

    // Methods
    writeLine("// Methods");
    writeLine("impl Method {");
    for (const request of requestsAndNotifications) {
        writeDoc(request.documentation, "    ");
        const methodName = methodNameIdentifier(request.method);
        writeLine(`    pub const ${methodName}: Method = Method(${rustStr(request.method)});`);
    }
    // Emit constants for registration-only methods (not also a request/notification)
    for (const reg of registrationMethods) {
        if (reg.isRegistrationOnly) {
            writeLine(`    // Registration-only method for ${reg.registrationMethod}.`);
            writeLine(`    pub const ${reg.fieldName}: Method = Method(${rustStr(reg.registrationMethod)});`);
        }
    }
    writeLine("}");
    writeLine("");

    // Go's Method(method) conversion of a decoded string, matched against the known methods.
    writeLine("impl Method {");
    writeLine("    pub fn from_str(s: &str) -> Method {");
    writeLine("        match s {");
    const knownMethods = new Set<string>();
    for (const request of requestsAndNotifications) {
        if (knownMethods.has(request.method)) continue;
        knownMethods.add(request.method);
        writeLine(`            ${rustStr(request.method)} => Method::${methodNameIdentifier(request.method)},`);
    }
    for (const reg of registrationMethods) {
        if (reg.isRegistrationOnly && !knownMethods.has(reg.registrationMethod)) {
            knownMethods.add(reg.registrationMethod);
            writeLine(`            ${rustStr(reg.registrationMethod)} => Method::${reg.fieldName},`);
        }
    }
    writeLine("            _ => Method(intern(s)),");
    writeLine("        }");
    writeLine("    }");
    writeLine("}");
    writeLine("");

    function rustTypeOfResolved(t: GoType): string {
        return renderType(rustOfGo(t.name));
    }

    // Generate request response types
    writeLine("// Request response types");
    writeLine("");

    for (const request of requestsAndNotifications) {
        const methodName = methodNameIdentifier(request.method);

        let responseTypeName: string | undefined;

        if ("result" in request) {
            if (request.typeName && request.typeName.endsWith("Request")) {
                responseTypeName = request.typeName.replace(/Request$/, "Response");
            }
            else {
                responseTypeName = `${methodName}Response`;
            }

            writeLine(`// Response type for \`${request.method}\``);

            // Special case for response types that are explicitly base type "null"
            if (request.result.kind === "base" && request.result.name === "null") {
                writeLine(`pub type ${responseTypeName} = Null;`);
            }
            else {
                writeLine(`pub type ${responseTypeName} = ${rustTypeOfResolved(resolveType(request.result))};`);
            }
            writeLine("");
        }

        if (Array.isArray(request.params)) {
            throw new Error("Unexpected request params for " + methodName + ": " + JSON.stringify(request.params));
        }

        const paramType = request.params ? rustTypeOfResolved(resolveType(request.params)) : "NoParams";

        writeLine(`// Type mapping info for \`${request.method}\``);
        if (responseTypeName) {
            writeLine(`pub const ${screaming(methodName + "Info")}: RequestInfo<${paramType}, ${responseTypeName}> = RequestInfo::new(Method::${methodName});`);
        }
        else {
            writeLine(`pub const ${screaming(methodName + "Info")}: NotificationInfo<${paramType}> = NotificationInfo::new(Method::${methodName});`);
        }
        writeLine("");
    }

    // Generate type aliases
    writeLine("// Type aliases");
    writeLine("");
    for (const aliasName of customTypeAliases) {
        writeLine(`pub type ${aliasName.name} = ${rustTypeOfResolved(resolveType(aliasName.type))};`);
        writeLine("");
    }

    // Generate union types
    writeLine("// Union types");
    writeLine("");

    for (const [name, members] of typeInfo.unionTypes.entries()) {
        const fields = unionFields.get(name)!;
        const arms = unionArms.get(name)!;
        const rustField = new Map(arms.map((arm, i) => [arm.fieldName, fields[i].rust]));

        writeLine(derives(name));
        writeLine(`pub struct ${name} {`);
        let hasLocations = false;
        for (const f of fields) {
            writeLine(`    pub ${f.rust}: ${renderType(f.type)},`);
            if (f.goField === "Locations" && f.goType === "*[]Location") {
                hasLocations = true;
            }
        }
        writeLine(`}`);
        writeLine("");

        // Get the field names and types for marshal/unmarshal methods
        const fieldEntries: UnionArm[] = arms.map(arm => ({ ...arm }));

        // Marshal method
        const unionContainedNull = members.some(member => member.containedNull);
        writeLine(`impl Json for ${name} {`);
        writeLine(`    const GO_TYPE: &'static str = "lsproto.${name}";`);
        writeLine("");
        writeLine(`    fn to_json(&self) -> Value {`);
        writeLine(`        let count = ${fields.map(f => `self.${f.rust}.is_some() as usize`).join(" + ")};`);
        if (unionContainedNull) {
            writeLine(`        assert_at_most_one(${rustStr(`more than one element of ${name} is set`)}, count);`);
        }
        else {
            writeLine(`        assert_only_one(${rustStr(`exactly one element of ${name} should be set`)}, count);`);
        }
        for (const f of fields) {
            writeLine(`        if let Some(v) = &self.${f.rust} {`);
            writeLine(`            return v.to_json();`);
            writeLine(`        }`);
        }
        writeLine(`        Value::Null`);
        writeLine(`    }`);
        writeLine("");

        // Unmarshal method
        writeLine(`    fn from_json(v: &Value) -> Result<Self, JsonError> {`);
        writeLine(`        let mut o = Self::default();`);

        const errInvalidValueExpr = `Err(JsonError::method(Self::GO_TYPE, err_invalid_value(${rustStr(name)}, v)))`;
        const errInvalidKindExpr = `Err(JsonError::method(Self::GO_TYPE, err_invalid_kind(${rustStr(name)}, k)))`;

        function armAssign(entry: UnionArm, indent: string, expr: string) {
            writeLine(`${indent}o.${rustField.get(entry.fieldName)} = ${expr};`);
            writeLine(`${indent}return Ok(o);`);
        }

        // Try-each decoding of the given entries (Go's speculative json.Unmarshal).
        function tryEach(entries: UnionArm[], indent: string) {
            for (const entry of entries) {
                writeLine(`${indent}if let Ok(x) = Json::from_json(v) {`);
                writeLine(`${indent}    o.${rustField.get(entry.fieldName)} = Some(x);`);
                writeLine(`${indent}    return Ok(o);`);
                writeLine(`${indent}}`);
            }
        }

        function generateDiscriminatorDispatch(
            disc: NonNullable<ReturnType<typeof findDiscriminatorField>>,
            indent: string,
        ): boolean {
            writeLine(`${indent}match json_object_raw_field(v, ${rustStr(disc.fieldName)}).and_then(value_str) {`);
            for (const [value, entry] of disc.mapping) {
                writeLine(`${indent}    Some(${rustStr(value)}) => {`);
                armAssign(entry, indent + "        ", "arm_buffered(v)?");
                writeLine(`${indent}    }`);
            }
            let exhaustive = false;
            if (disc.unmapped.length > 0) {
                writeLine(`${indent}    _ => {`);
                exhaustive = generateUnmappedFallback(disc.unmapped, indent + "        ");
                writeLine(`${indent}    }`);
            }
            else {
                writeLine(`${indent}    _ => {}`);
            }
            writeLine(`${indent}}`);
            return exhaustive;
        }

        function generateStreamingDiscriminatorDispatch(
            disc: NonNullable<ReturnType<typeof findDiscriminatorField>>,
            indent: string,
        ) {
            writeLine(`${indent}let state = scan_discriminated_struct(v, Self::GO_TYPE, ${rustStr(name)}, ${rustStr(disc.fieldName)})?;`);
            writeLine(`${indent}match state.discriminator_str() {`);
            for (const [value, entry] of disc.mapping) {
                writeLine(`${indent}    Some(${rustStr(value)}) => {`);
                armAssign(entry, indent + "        ", `Some(unmarshal_discriminated_arm(v, Self::GO_TYPE, ${rustStr(disc.fieldName)})?)`);
                writeLine(`${indent}    }`);
            }
            if (disc.unmapped.length === 1) {
                writeLine(`${indent}    _ => {`);
                armAssign(disc.unmapped[0], indent + "        ", `Some(unmarshal_discriminated_arm(v, Self::GO_TYPE, ${rustStr(disc.fieldName)})?)`);
                writeLine(`${indent}    }`);
            }
            else {
                writeLine(`${indent}    _ => return Err(state.invalid_discriminator(Self::GO_TYPE)),`);
            }
            writeLine(`${indent}}`);
        }

        function canStreamDiscriminator(
            disc: NonNullable<ReturnType<typeof findDiscriminatorField>>,
        ): boolean {
            if (disc.unmapped.length > 1) {
                return false;
            }
            const entries = [...disc.mapping.values(), ...disc.unmapped];
            return entries.every(entry => {
                if (entry.originalType.kind !== "reference") {
                    return false;
                }
                const name = entry.originalType.name;
                return !hasCustomStructureCodec(name) && model.structures.some(structure => structure.name === name);
            });
        }

        function generateUnmappedFallback(unmapped: UnionArm[], indent: string): boolean {
            if (unmapped.length <= 1) {
                // Exactly 1 entry: it's the only remaining variant after dispatch,
                // so use a hard error return instead of speculative decoding.
                for (const entry of unmapped) {
                    armAssign(entry, indent, "arm_buffered(v)?");
                }
                return unmapped.length === 1;
            }
            // Try chaining presence dispatch on the remaining subset
            const pres = findPresenceDiscriminator(unmapped);
            if (pres) {
                return generatePresenceDispatch(pres, indent);
            }
            else {
                tryEach(unmapped, indent);
                return false;
            }
        }

        function collectAllPresenceChecks(
            pres: NonNullable<ReturnType<typeof findPresenceDiscriminator>>,
        ) {
            const allChecks = [...pres.checks];
            let remaining = pres.unmapped;
            while (remaining.length > 1) {
                const next = findPresenceDiscriminator(remaining);
                if (!next) break;
                allChecks.push(...next.checks);
                remaining = next.unmapped;
            }
            return { allChecks, finalUnmapped: remaining };
        }

        function generatePresenceDispatch(
            pres: NonNullable<ReturnType<typeof findPresenceDiscriminator>>,
            indent: string,
        ): boolean {
            const { allChecks, finalUnmapped } = collectAllPresenceChecks(pres);
            const args = allChecks.map(c => rustStr(c.jsonFieldName)).join(", ");
            writeLine(`${indent}match json_object_has_key(v, &[${args}]) {`);
            for (let i = 0; i < allChecks.length; i++) {
                writeLine(`${indent}    // ${allChecks[i].jsonFieldName}`);
                writeLine(`${indent}    ${i} => {`);
                armAssign(allChecks[i].entry, indent + "        ", "arm_buffered(v)?");
                writeLine(`${indent}    }`);
            }
            if (finalUnmapped.length > 0) {
                writeLine(`${indent}    _ => {`);
                if (finalUnmapped.length === 1) {
                    // Only one variant left after dispatch — use hard error return.
                    armAssign(finalUnmapped[0], indent + "        ", "arm_buffered(v)?");
                }
                else {
                    tryEach(finalUnmapped, indent + "        ");
                }
                writeLine(`${indent}    }`);
            }
            else {
                writeLine(`${indent}    _ => {}`);
            }
            writeLine(`${indent}}`);
            // Exhaustive if the default case has a single hard-returning entry
            return finalUnmapped.length === 1;
        }

        // Group field entries by their expected JSON token kind for optimized dispatch.
        const kindMap = new Map<string, typeof fieldEntries>();
        const unknownKindEntries: typeof fieldEntries = [];
        for (const entry of fieldEntries) {
            const kind = jsonKindForType(entry.originalType);
            if (!kind) {
                unknownKindEntries.push(entry);
            }
            else {
                if (!kindMap.has(kind)) kindMap.set(kind, []);
                kindMap.get(kind)!.push(entry);
            }
        }

        // Sort ambiguous variants (same JSON kind) by number of required fields
        // descending, so more specific variants are tried first.
        function countRequiredFields(entry: typeof fieldEntries[0]): number {
            if (entry.originalType.kind !== "reference") return 0;
            const structure = model.structures.find(s => s.name === (entry.originalType as ReferenceType).name);
            if (!structure) return 0;
            return structure.properties.filter(p => !p.optional && !p.omitzeroValue).length;
        }

        for (const [, entries] of kindMap) {
            if (entries.length > 1) {
                entries.sort((a, b) => countRequiredFields(b) - countRequiredFields(a));
            }
        }

        // Also sort the flat fieldEntries to match (for the fallback path)
        {
            const sorted: typeof fieldEntries = [];
            const seen = new Set<string>();
            for (const [, entries] of kindMap) {
                for (const entry of entries) {
                    sorted.push(entry);
                    seen.add(entry.fieldName);
                }
            }
            for (const entry of unknownKindEntries) {
                if (!seen.has(entry.fieldName)) {
                    sorted.push(entry);
                }
            }
            fieldEntries.length = 0;
            fieldEntries.push(...sorted);
        }

        const hasUnknownKinds = unknownKindEntries.length > 0;
        const distinctKinds = kindMap.size + (unionContainedNull ? 1 : 0);
        const canDispatch = !hasUnknownKinds && distinctKinds >= 2;
        const allUnambiguous = canDispatch && Array.from(kindMap.values()).every(entries => entries.length === 1);

        function kindPattern(kind: string): string {
            switch (kind) {
                case "string":
                    return `b'"'`;
                case "number":
                    return `b'0'`;
                case "object":
                    return `b'{'`;
                case "array":
                    return `b'['`;
                case "boolean":
                    return `b't' | b'f'`;
                default:
                    throw new Error(`unexpected kind ${kind}`);
            }
        }

        function unambiguousArm(kind: string, entry: UnionArm, indent: string) {
            if (kind === "boolean") {
                armAssign(entry, indent, "Some(k == b't')");
            }
            else {
                armAssign(entry, indent, "arm(v)?");
            }
        }

        let fallbackExhaustive = false;
        if (canDispatch) {
            writeLine(`        let k = kind(v);`);
            writeLine(`        match k {`);
            if (unionContainedNull) {
                writeLine(`            b'n' => Ok(o),`);
            }
            for (const [kind, entries] of kindMap) {
                writeLine(`            ${kindPattern(kind)} => {`);
                if (allUnambiguous || entries.length === 1) {
                    unambiguousArm(kind, entries[0], "                ");
                }
                else {
                    let exhaustive = false;
                    const disc = findDiscriminatorField(entries);
                    if (disc && canStreamDiscriminator(disc)) {
                        generateStreamingDiscriminatorDispatch(disc, "                ");
                        exhaustive = true;
                    }
                    if (disc && !canStreamDiscriminator(disc)) {
                        exhaustive = generateDiscriminatorDispatch(disc, "                ");
                    }
                    else if (!disc) {
                        const pres = findPresenceDiscriminator(entries);
                        if (pres) {
                            exhaustive = generatePresenceDispatch(pres, "                ");
                        }
                        else {
                            tryEach(entries, "                ");
                        }
                    }
                    if (!exhaustive) {
                        writeLine(`                ${errInvalidValueExpr}`);
                    }
                }
                writeLine(`            }`);
            }
            writeLine(`            _ => ${errInvalidKindExpr},`);
            writeLine(`        }`);
        }
        else {
            // Fallback for unknown kinds (e.g. `any`). Discriminated object
            // unions can still stream; other unions use try-each.
            let exhaustive = false;
            const disc = findDiscriminatorField(fieldEntries);
            if (disc && canStreamDiscriminator(disc)) {
                generateStreamingDiscriminatorDispatch(disc, "        ");
                exhaustive = true;
            }
            else {
                if (unionContainedNull) {
                    writeLine(`        if is_null(v) {`);
                    writeLine(`            return Ok(o);`);
                    writeLine(`        }`);
                }
            }
            if (disc && !canStreamDiscriminator(disc)) {
                exhaustive = generateDiscriminatorDispatch(disc, "        ");
            }
            else if (!disc) {
                const pres = findPresenceDiscriminator(fieldEntries);
                if (pres) {
                    exhaustive = generatePresenceDispatch(pres, "        ");
                }
                else {
                    tryEach(fieldEntries, "        ");
                }
            }
            fallbackExhaustive = exhaustive;
            if (!fallbackExhaustive) {
                writeLine(`        ${errInvalidValueExpr}`);
            }
        }
        writeLine(`    }`);
        writeLine(`}`);
        writeLine("");

        // Generate GetLocations method
        if (hasLocations) {
            writeLine(`impl HasLocations for ${name} {`);
            writeLine(`    fn get_locations(&self) -> Option<&Vec<Location>> {`);
            writeLine(`        self.locations.as_ref()`);
            writeLine(`    }`);
            writeLine(`}`);
            writeLine("");
        }
    }

    // Generate literal types
    writeLine("// Literal types");
    writeLine("");

    for (const [value, name] of typeInfo.literalTypes.entries()) {
        const jsonValue = JSON.stringify(value);

        writeLine(`// ${name} is a literal type for ${jsonValue}`);
        writeLine(`#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]`);
        writeLine(`pub struct ${name};`);
        writeLine("");
        writeLine(`impl Json for ${name} {`);
        writeLine(`    const GO_TYPE: &'static str = "lsproto.${name}";`);
        writeLine("");
        writeLine(`    fn to_json(&self) -> Value {`);
        writeLine(`        Value::String(${rustStr(value)}.to_string())`);
        writeLine(`    }`);
        writeLine("");
        writeLine(`    fn from_json(v: &Value) -> Result<Self, JsonError> {`);
        writeLine(`        if !matches!(v, Value::String(s) if s == ${rustStr(value)}) {`);
        writeLine(`            return Err(JsonError::method(Self::GO_TYPE, err_literal_mismatch(${rustStr(name)}, ${rustStr(jsonValue)}, v)));`);
        writeLine(`        }`);
        writeLine(`        Ok(${name})`);
        writeLine(`    }`);
        writeLine(`}`);
        writeLine("");
    }

    // Generate resolved capabilities
    const clientCapsStructure = model.structures.find(s => s.name === "ClientCapabilities");
    if (clientCapsStructure) {
        function generateResolvedStruct(structure: Structure, indent: string): string[] {
            const lines: string[] = [];
            for (const prop of structure.properties) {
                if (prop.documentation) {
                    const propDoc = formatDocumentation(prop.documentation);
                    if (propDoc) {
                        for (const line of propDoc.split("\n").filter(l => l)) {
                            lines.push(`${indent}${line}`);
                        }
                    }
                }

                const type = resolveType(prop.type);
                const fieldName = rustIdent(goFieldName(prop));

                if (prop.type.kind === "reference") {
                    const refStructure = model.structures.find(s => s.name === type.name);
                    if (refStructure) {
                        lines.push(`${indent}pub ${fieldName}: Resolved${type.name},`);
                        continue;
                    }
                }

                lines.push(`${indent}pub ${fieldName}: ${rustTypeOfResolved(type)},`);
            }
            return lines;
        }

        function generateResolveConversion(structure: Structure, indent: string): string[] {
            const lines: string[] = [];
            const fields = structFields.get(structure.name)!;
            for (let i = 0; i < structure.properties.length; i++) {
                const prop = structure.properties[i];
                const f = fields[i];
                const type = resolveType(prop.type);

                if (prop.type.kind === "reference") {
                    const refStructure = model.structures.find(s => s.name === type.name);
                    if (refStructure) {
                        if (f.type.k === "opt") {
                            lines.push(`${indent}${f.rust}: self.${f.rust}.as_ref().map(|v| v.resolve()).unwrap_or_default(),`);
                        }
                        else {
                            lines.push(`${indent}${f.rust}: self.${f.rust}.resolve(),`);
                        }
                        continue;
                    }
                }

                if (f.type.k === "opt") {
                    lines.push(`${indent}${f.rust}: self.${f.rust}.clone().unwrap_or_default(),`);
                }
                else {
                    lines.push(`${indent}${f.rust}: self.${f.rust}.clone(),`);
                }
            }
            return lines;
        }

        function collectStructureDependencies(structure: Structure, visited = new Set<string>()): Structure[] {
            if (visited.has(structure.name)) {
                return [];
            }
            visited.add(structure.name);

            const deps: Structure[] = [];

            for (const prop of structure.properties) {
                if (prop.type.kind === "reference") {
                    const refStructure = model.structures.find(s => s.name === (prop.type as ReferenceType).name);
                    if (refStructure) {
                        deps.push(...collectStructureDependencies(refStructure, new Set(visited)));
                        deps.push(refStructure);
                    }
                }
            }

            return deps;
        }

        function generateResolvedTypeAndHelper(structure: Structure, isMain: boolean = false): string[] {
            const lines: string[] = [];
            const typeName = `Resolved${structure.name}`;
            // Main method is exported (Resolve), helpers are unexported (resolve)
            const visibility = isMain ? `pub` : `pub(crate)`;

            if (!isMain) {
                if (structure.documentation) {
                    const typeDoc = formatDocumentation(structure.documentation);
                    if (typeDoc) {
                        lines.push(`// ${typeName} is a resolved version of ${structure.name} with all optional fields`);
                        lines.push(`// converted to non-pointer values for easier access.`);
                        lines.push(`//`);
                        for (const line of typeDoc.split("\n").filter(l => l)) {
                            lines.push(line);
                        }
                    }
                }
                else {
                    lines.push(`// ${typeName} is a resolved version of ${structure.name} with all optional fields`);
                    lines.push(`// converted to non-pointer values for easier access.`);
                }
            }

            lines.push(`#[derive(Clone, Debug, Default, PartialEq)]`);
            lines.push(`pub struct ${typeName} {`);
            lines.push(...generateResolvedStruct(structure, "    "));
            lines.push(`}`);
            lines.push(``);

            lines.push(`impl ${structure.name} {`);
            lines.push(`    ${visibility} fn resolve(&self) -> ${typeName} {`);
            lines.push(`        ${typeName} {`);
            lines.push(...generateResolveConversion(structure, "            "));
            lines.push(`        }`);
            lines.push(`    }`);
            lines.push(`}`);
            lines.push(``);

            return lines;
        }

        // Collect all dependent structures and generate their resolved types
        const deps = collectStructureDependencies(clientCapsStructure);
        const uniqueDeps = Array.from(new Map(deps.map(d => [d.name, d])).values());

        for (const dep of uniqueDeps) {
            for (const line of generateResolvedTypeAndHelper(dep, false)) {
                writeLine(line);
            }
        }

        // Generate the main ResolvedClientCapabilities type and function
        writeLine("// ResolvedClientCapabilities is a version of ClientCapabilities where all nested");
        writeLine("// fields are values (not pointers), making it easier to access deeply nested capabilities.");
        writeLine("// Use ClientCapabilities::resolve() to convert from ClientCapabilities.");
        if (clientCapsStructure.documentation) {
            writeLine("//");
            const typeDoc = formatDocumentation(clientCapsStructure.documentation);
            for (const line of typeDoc.split("\n").filter(l => l)) {
                writeLine(line);
            }
        }
        for (const line of generateResolvedTypeAndHelper(clientCapsStructure, true)) {
            writeLine(line);
        }
    }

    let code = parts.join("");
    // No blank line before a closing brace or at the end of the file.
    code = code.replace(/[ \t]+\n/g, "\n").replace(/\n\n+(\s*\})/g, "\n$1").replace(/\n+$/, "\n");
    return code;
}

function hasSomeProp(structure: Structure, propName: string, propTypeName: string) {
    return structure.properties?.some(p =>
        !p.optional &&
        p.name === propName &&
        p.type.kind === "reference" &&
        p.type.name === propTypeName
    );
}

function hasTextDocumentURI(structure: Structure) {
    return hasSomeProp(structure, "textDocument", "TextDocumentIdentifier") ||
        hasSomeProp(structure, "_vs_textDocument", "TextDocumentIdentifier");
}

function hasTextDocumentPosition(structure: Structure) {
    return hasSomeProp(structure, "position", "Position") ||
        hasSomeProp(structure, "_vs_position", "Position");
}

function getLocationUriProperty(structure: Structure) {
    const prop = structure.properties?.find(p =>
        !p.optional &&
        titleCase(p.name).endsWith("Uri") &&
        p.type.kind === "base" &&
        p.type.name === "DocumentUri"
    );
    if (
        prop &&
        structure.properties.some(p =>
            !p.optional &&
            titleCase(p.name) === titleCase(prop.name).replace(/Uri$/, "Range") &&
            p.type.kind === "reference" &&
            p.type.name === "Range"
        )
    ) {
        return titleCase(prop.name);
    }
}

/**
 * Main function
 */
export default async function generate() {
    collectTypeDefinitions();
    const generatedCode = generateCode();
    fs.writeFileSync(out, generatedCode);
    console.log(`Successfully generated ${out}`);
}

if (process.argv[1] === __filename) {
    await generate();
}
