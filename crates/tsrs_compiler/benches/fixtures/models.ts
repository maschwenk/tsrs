// Benchmark fixture: a small, self-contained domain model that exercises common checker paths
// (interfaces, generics, unions, mapped and conditional types, classes, overloads, narrowing).

export type Id<Tag extends string> = string & { readonly __tag: Tag };

export interface Entity<Tag extends string> {
    readonly id: Id<Tag>;
    createdAt: number;
    updatedAt?: number;
}

export interface User extends Entity<"user"> {
    name: string;
    email: string;
    roles: Role[];
    profile?: Profile;
}

export interface Profile {
    bio: string;
    links: { label: string; url: string }[];
    settings: Partial<Settings>;
}

export interface Settings {
    theme: "light" | "dark" | "system";
    notifications: { email: boolean; push: boolean; digest: "daily" | "weekly" | "never" };
    locale: string;
}

export type Role = "admin" | "editor" | "viewer";

export interface Post extends Entity<"post"> {
    author: Id<"user">;
    title: string;
    body: string;
    tags: string[];
    status: PostStatus;
}

export type PostStatus =
    | { kind: "draft" }
    | { kind: "scheduled"; at: number }
    | { kind: "published"; at: number; views: number }
    | { kind: "archived"; reason: string };

export type DeepReadonly<T> = T extends (infer E)[]
    ? ReadonlyArray<DeepReadonly<E>>
    : T extends object
      ? { readonly [K in keyof T]: DeepReadonly<T[K]> }
      : T;

export type PickByValue<T, V> = { [K in keyof T as T[K] extends V ? K : never]: T[K] };

export type StringFields<T> = keyof PickByValue<T, string>;

export type Patch<T> = { [K in keyof T]?: T[K] extends object ? Patch<T[K]> : T[K] };

export function makeId<Tag extends string>(tag: Tag, n: number): Id<Tag> {
    return (tag + ":" + n.toString(36)) as Id<Tag>;
}

export function isPublished(status: PostStatus): status is Extract<PostStatus, { kind: "published" }> {
    return status.kind === "published";
}

export function describeStatus(status: PostStatus): string {
    switch (status.kind) {
        case "draft":
            return "draft";
        case "scheduled":
            return "scheduled for " + new Date(status.at).toUTCString();
        case "published":
            return "published (" + status.views + " views)";
        case "archived":
            return "archived: " + status.reason;
    }
}
