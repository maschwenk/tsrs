interface RequiredEnv {
    NODE_ENV: string;
}
// Closes the cycle ProcessEnv -> Env -> ProcessEnv. The member entered first drops its base in the cycle: entered at
// Env, ProcessEnv inherits Env's optional NODE_ENV; entered at ProcessEnv, it inherits RequiredEnv's required one.
interface ProcessEnv extends Env, RequiredEnv {}
