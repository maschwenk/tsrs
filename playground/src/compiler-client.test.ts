import { expect, test, vi } from "vite-plus/test";
import { CompilerClient, type CompilerTransport } from "./compiler-client";
import { compilerDefaults, type Compilation, type CompileRequest } from "./project";

function transport() {
  const worker: CompilerTransport = {
    postMessage: vi.fn(),
    onmessage: null,
    onerror: null,
    terminate: vi.fn(),
  };
  const reply = (revision: number, error?: string) =>
    worker.onmessage?.(
      new MessageEvent<Compilation>("message", {
        data: {
          revision,
          error,
          diagnostics: [],
          files: {},
          sourceFiles: [],
          exitCode: error ? 5 : 0,
          stderr: "",
        },
      }),
    );
  return { worker, reply };
}
const input = (source: string): Omit<CompileRequest, "revision"> => ({
  source,
  fileName: "input.ts",
  compilerOptions: compilerDefaults,
  libraries: {},
});

test("typing during a run drops superseded work and never publishes stale results", async () => {
  const { worker, reply } = transport();
  const client = new CompilerClient(() => worker);
  const publish = vi.fn();
  client.subscribe(publish);
  client.request(input("first"));
  const ready = client.ready();
  client.request(input("second"));
  client.request(input("latest"));
  reply(1);
  expect(publish).not.toHaveBeenCalled();
  expect(worker.postMessage).toHaveBeenCalledTimes(2);
  expect(worker.postMessage).toHaveBeenLastCalledWith(
    expect.objectContaining({ revision: 3, source: "latest" }),
  );
  reply(3);
  expect((await ready).revision).toBe(3);
  expect(publish).toHaveBeenCalledTimes(1);
  client.dispose();
});

test("a failed worker clears outputs and is replaced for the next run", async () => {
  const first = transport();
  const second = transport();
  const create = vi.fn().mockReturnValueOnce(first.worker).mockReturnValue(second.worker);
  const client = new CompilerClient(create);
  client.request(input("code"));
  const ready = client.ready();
  first.worker.onerror?.({ message: "worker failed", preventDefault() {} } as ErrorEvent);
  expect(await ready).toMatchObject({ error: "worker failed", files: {}, diagnostics: [] });
  expect(first.worker.terminate).toHaveBeenCalled();
  client.request(input("retry"));
  second.reply(2);
  expect((await client.ready()).error).toBeUndefined();
  client.dispose();
});

test("compiler traps are delivered as recoverable results", async () => {
  const { worker, reply } = transport();
  const client = new CompilerClient(() => worker);
  client.request(input("deep expression"));
  reply(1, "WASM trapped");
  expect(await client.ready()).toMatchObject({ error: "WASM trapped", diagnostics: [], files: {} });
  client.request(input("valid code"));
  reply(2);
  expect((await client.ready()).exitCode).toBe(0);
  client.dispose();
});

test("editing invalidates an in-flight result before the debounce interval ends", () => {
  vi.useFakeTimers();
  const { worker, reply } = transport();
  const client = new CompilerClient(() => worker);
  const publish = vi.fn();
  client.subscribe(publish);
  client.request(input("old"));
  client.request(input("new"), 300);
  reply(1);
  expect(publish).not.toHaveBeenCalled();
  expect(worker.postMessage).toHaveBeenCalledTimes(1);
  vi.advanceTimersByTime(300);
  expect(worker.postMessage).toHaveBeenCalledTimes(2);
  reply(2);
  expect(publish).toHaveBeenCalledTimes(1);
  client.dispose();
  vi.useRealTimers();
});
