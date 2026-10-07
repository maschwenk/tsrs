// The first declaration of the cycle in program order.
interface ProcessEnv {
    [key: string]: string | undefined;
    TZ?: string;
}
