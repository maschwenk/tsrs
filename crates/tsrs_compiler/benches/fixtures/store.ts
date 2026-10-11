// Benchmark fixture: generic repository and event plumbing over the models.

import { Entity, Id, Patch, Post, PostStatus, Role, User, describeStatus, isPublished, makeId } from "./models";

export type Listener<E> = (event: E) => void;

export class Emitter<Events extends Record<string, unknown>> {
    private listeners: { [K in keyof Events]?: Listener<Events[K]>[] } = {};

    on<K extends keyof Events>(name: K, listener: Listener<Events[K]>): () => void {
        const list = this.listeners[name] || (this.listeners[name] = []);
        list.push(listener);
        return () => {
            const index = list.indexOf(listener);
            if (index >= 0) {
                list.splice(index, 1);
            }
        };
    }

    emit<K extends keyof Events>(name: K, event: Events[K]): void {
        const list = this.listeners[name];
        if (list) {
            for (const listener of list.slice()) {
                listener(event);
            }
        }
    }
}

export interface StoreEvents<T> {
    added: T;
    updated: { before: T; after: T };
    removed: T;
    [key: string]: unknown;
}

export class Repository<Tag extends string, T extends Entity<Tag>> extends Emitter<StoreEvents<T>> {
    private items: { [id: string]: T } = {};
    private order: Id<Tag>[] = [];

    add(item: T): T {
        this.items[item.id] = item;
        this.order.push(item.id);
        this.emit("added", item);
        return item;
    }

    get(id: Id<Tag>): T | undefined {
        return this.items[id];
    }

    update(id: Id<Tag>, patch: Patch<T>, now: number): T | undefined {
        const before = this.get(id);
        if (!before) {
            return undefined;
        }
        const after = merge(before, patch);
        after.updatedAt = now;
        this.items[id] = after;
        this.emit("updated", { before, after });
        return after;
    }

    remove(id: Id<Tag>): boolean {
        const item = this.get(id);
        if (item === undefined) {
            return false;
        }
        delete this.items[id];
        this.order = this.order.filter(other => other !== id);
        this.emit("removed", item);
        return true;
    }

    all(): T[] {
        return this.order.map(id => this.items[id]);
    }

    where<K extends keyof T>(key: K, value: T[K]): T[] {
        return this.all().filter(item => item[key] === value);
    }

    reduce<R>(fn: (acc: R, item: T, index: number) => R, initial: R): R {
        return this.all().reduce(fn, initial);
    }
}

function merge<T>(target: T, patch: Patch<T>): T {
    const result = { ...target } as any;
    for (const key in patch) {
        const value = (patch as any)[key];
        if (value !== null && typeof value === "object" && !Array.isArray(value)) {
            result[key] = merge(result[key], value);
        } else if (value !== undefined) {
            result[key] = value;
        }
    }
    return result as T;
}

export function hasRole(user: User, ...roles: Role[]): boolean {
    return user.roles.some(role => roles.indexOf(role) >= 0);
}

export function summarize(users: Repository<"user", User>, posts: Repository<"post", Post>): string[] {
    const lines: string[] = [];
    for (const user of users.all()) {
        const own = posts.where("author", user.id);
        const published = own.filter(post => isPublished(post.status));
        const views = published.reduce((sum, post) => sum + (post.status.kind === "published" ? post.status.views : 0), 0);
        lines.push(user.name + ": " + own.length + " posts, " + published.length + " published, " + views + " views");
        for (const post of own) {
            lines.push("  " + post.title + " [" + describeStatus(post.status) + "]");
        }
    }
    return lines;
}

export function seed(count: number): { users: Repository<"user", User>; posts: Repository<"post", Post> } {
    const users = new Repository<"user", User>();
    const posts = new Repository<"post", Post>();
    for (let i = 0; i < count; i++) {
        const user = users.add({
            id: makeId("user", i),
            createdAt: i,
            name: "user" + i,
            email: "user" + i + "@example.com",
            roles: i % 3 === 0 ? ["admin"] : ["viewer"],
        });
        const statuses: PostStatus[] = [{ kind: "draft" }, { kind: "published", at: i, views: i * 10 }, { kind: "archived", reason: "old" }];
        for (let j = 0; j < statuses.length; j++) {
            posts.add({ id: makeId("post", i * 10 + j), createdAt: i, author: user.id, title: "post " + j, body: "", tags: [], status: statuses[j] });
        }
    }
    users.update(makeId("user", 0), { profile: { bio: "first", links: [], settings: { theme: "dark" } } }, count);
    return { users, posts };
}
