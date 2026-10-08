type Box<T extends string> = { value: T };
const make = (): Box<number> => ({ value: 1 }) as any;
