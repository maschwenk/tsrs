import type { Compilation, CompileRequest } from "./project";

export interface CompilerTransport {
  postMessage: (request: CompileRequest) => void;
  onmessage: ((event: MessageEvent<Compilation>) => void) | null;
  onerror: ((event: ErrorEvent) => void) | null;
  terminate: () => void;
}

/** Serializes runs, retaining just the newest pending editor revision. */
export class CompilerClient {
  private worker: CompilerTransport;
  private pending?: CompileRequest;
  private active?: CompileRequest;
  private revision = 0;
  private result?: Compilation;
  private waiters: Array<(result: Compilation) => void> = [];
  private listeners = new Set<(result: Compilation) => void>();
  private readonly createWorker: () => CompilerTransport;
  private timer?: ReturnType<typeof setTimeout>;
  private needsWorker = false;

  constructor(createWorker: () => CompilerTransport) {
    this.createWorker = createWorker;
    this.worker = this.connect();
  }

  private connect() {
    const worker = this.createWorker();
    worker.onmessage = ({ data }) => this.receive(data);
    worker.onerror = (event) => {
      if (worker !== this.worker) return;
      event.preventDefault();
      worker.terminate();
      this.needsWorker = true;
      this.receive({
        revision: this.active?.revision ?? this.revision,
        diagnostics: [],
        files: {},
        sourceFiles: [],
        exitCode: 5,
        stderr: "",
        error: event.message || "Compiler worker failed. Edit or retry to run again.",
      });
    };
    return worker;
  }

  request(input: Omit<CompileRequest, "revision">, delay = 0) {
    if (this.timer) clearTimeout(this.timer);
    this.timer = undefined;
    this.pending = { ...input, revision: ++this.revision };
    this.result = undefined;
    if (delay)
      this.timer = setTimeout(() => {
        this.timer = undefined;
        this.send();
      }, delay);
    else this.send();
  }

  private send() {
    if (this.active || !this.pending || this.timer) return;
    this.active = this.pending;
    this.pending = undefined;
    if (this.needsWorker) {
      this.worker = this.connect();
      this.needsWorker = false;
    }
    this.worker.postMessage(this.active);
  }

  private receive(result: Compilation) {
    if (result.revision !== this.active?.revision) return;
    this.active = undefined;
    if (result.revision === this.revision) {
      this.result = result;
      for (const listener of this.listeners) listener(result);
      for (const resolve of this.waiters.splice(0)) resolve(result);
    }
    this.send();
  }

  ready(): Promise<Compilation> {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = undefined;
      this.send();
    }
    return this.result
      ? Promise.resolve(this.result)
      : new Promise((resolve) => this.waiters.push(resolve));
  }

  subscribe(listener: (result: Compilation) => void): () => void {
    this.listeners.add(listener);
    if (this.result) listener(this.result);
    return () => this.listeners.delete(listener);
  }

  dispose() {
    if (this.timer) clearTimeout(this.timer);
    this.worker.terminate();
    this.listeners.clear();
    for (const resolve of this.waiters.splice(0))
      resolve({
        revision: this.revision,
        diagnostics: [],
        files: {},
        sourceFiles: [],
        exitCode: 5,
        stderr: "",
        error: "Compiler stopped.",
      });
  }
}
