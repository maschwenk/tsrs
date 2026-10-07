// Enters the cycle at ProcessEnv.
declare function spawn(options: { env?: ProcessEnv }): void;
spawn({ env: { ...env, EXTRA: "" } });
export {};
