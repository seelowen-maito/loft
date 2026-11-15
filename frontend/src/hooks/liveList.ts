import { useEffect, useSyncExternalStore } from "react";
import { onConnection, onServerEvent, type ServerEvent } from "../lib/events";
import { errorText } from "../lib/useAction";

/** A live list: null until its first load, `failed` with the reason when that load failed. */
export type ListState<T> = { items: T[] | null; failed: string | null };

/**
 * One shared copy of a list for every component that shows it: loaded once, kept current by `apply` on each server
 * event, and loaded again after a reconnect, as events sent while offline are lost. Events that arrive while it loads
 * are applied to what loads, as they are newer.
 */
export function createLiveList<T>(load: () => Promise<T[]>, apply: (items: T[], e: ServerEvent) => T[]) {
  let state: ListState<T> = { items: null, failed: null };
  let loading: Promise<void> | null = null;
  let queued: ServerEvent[] = [];
  let wanted = false;
  let online = false;
  let dropped = false;
  let attached = false;
  const listeners = new Set<() => void>();
  const put = (s: ListState<T>) => {
    state = s;
    listeners.forEach((l) => l());
  };

  const store = {
    get: () => state,
    subscribe(l: () => void) {
      listeners.add(l);
      return () => {
        listeners.delete(l);
      };
    },
    /** Loads it unless it's loaded or loading. */
    ensure() {
      wanted = true;
      if (!state.items && !loading) store.reload();
    },
    /** Loads it again; `from` reads it some other way (the Repos page's Refresh). */
    reload(from: () => Promise<T[]> = load): Promise<void> {
      wanted = true;
      queued = [];
      const p = from().then(
        (items) => {
          if (loading !== p) return;
          put({ items: queued.reduce(apply, items), failed: null });
        },
        (e) => {
          if (loading !== p) return;
          put({ items: state.items, failed: state.items ? null : errorText(e) });
        },
      ).finally(() => {
        if (loading !== p) return;
        loading = null;
        queued = [];
      });
      loading = p;
      return p;
    },
    event(e: ServerEvent) {
      if (loading) queued.push(e);
      if (!state.items) return;
      const items = apply(state.items, e);
      if (items !== state.items) put({ ...state, items });
    },
    /** The event stream opened or dropped: back after a drop, or after a first load that failed, it loads again. */
    connection(live: boolean) {
      if (live && wanted && (dropped || (state.failed && !loading))) store.reload();
      if (live) dropped = false;
      else if (online) dropped = true;
      online = live;
    },
    /** Changes the shared copy in place (an optimistic edit, such as a chat marked read). */
    set(fn: (items: T[]) => T[]) {
      if (state.items) put({ ...state, items: fn(state.items) });
    },
    /** The list in a component, loaded on first use. */
    use(): ListState<T> {
      useEffect(() => {
        if (!attached) {
          attached = true;
          onServerEvent(store.event);
          onConnection(store.connection);
        }
        store.ensure();
      }, []);
      return useSyncExternalStore(store.subscribe, store.get);
    },
  };
  return store;
}

/** `items` with `item` in place of the one with its id, or added first. */
export function upsert<T extends { id: string }>(items: T[], item: T): T[] {
  const i = items.findIndex((x) => x.id === item.id);
  if (i < 0) return [item, ...items];
  const next = [...items];
  next[i] = item;
  return next;
}
