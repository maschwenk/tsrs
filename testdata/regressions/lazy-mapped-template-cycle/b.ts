// The same cycle through a mapped type over an object type, which gets a lazy table.
export type Like = Exclude<string | LikeMap<{ a: string }>, any[]> | object;
export type LikeMap<T> = { [K in keyof T]?: Like | Like[] };
