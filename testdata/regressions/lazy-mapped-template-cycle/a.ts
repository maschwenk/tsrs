// joi's SchemaLike: the template of SchemaMap<any> reaches the conditional that relates SchemaMap<any> again.
export type SchemaLikeWithoutArray<T = any> = Exclude<string | Schema<T> | SchemaMap<T>, any[]>;
export type SchemaLike<T = any> = SchemaLikeWithoutArray<T> | object;
export type SchemaMap<T = any> = { [K in keyof T]?: SchemaLike | SchemaLike[] };

export interface Schema<T = any> {
  items<I, U extends SchemaLikeWithoutArray<I>[] = SchemaLikeWithoutArray<I>[]>(...items: U): Schema<I[]>;
}
