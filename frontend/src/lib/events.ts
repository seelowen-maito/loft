import { useEffect, useSyncExternalStore } from "react";
import type { Agent, GrillSession, Message, Task, TaskComment, PermissionRequest, PullRequest, SystemReading, UpdateStatus } from "./api";

export type ServerEvent =
  | { type: "agent_updated"; agent: Agent }
  | { type: "agent_deleted"; agent_id: string }
  | { type: "message"; message: Message }
  | { type: "permission"; request: PermissionRequest }
  | { type: "memory"; agent_id: string }
  | { type: "pull_request"; pr: PullRequest }
  | { type: "update"; status: UpdateStatus }
  | { type: "system"; system: SystemReading }
  | { type: "grill"; session: GrillSession }
  | { type: "task"; task: Task }
  | { type: "task_deleted"; task_id: string }
  | { type: "task_comment"; comment: TaskComment };

type Listener = (e: ServerEvent) => void;

const listeners = new Set<Listener>();
const statusListeners = new Set<() => void>();
let connected = false;
let source: EventSource | null = null;

function setConnected(v: boolean) {
  if (connected !== v) {
    connected = v;
    statusListeners.forEach((l) => l());
  }
}

function connect() {
  if (source) return;
  source = new EventSource("/api/events");
  source.onopen = () => setConnected(true);
  source.onerror = () => setConnected(false);
  source.onmessage = (m) => {
    try {
      const ev = JSON.parse(m.data) as ServerEvent;
      listeners.forEach((l) => l(ev));
    } catch {
      /* ignore malformed */
    }
  };
}

/** Subscribe to the backend's live event stream. */
export function useServerEvents(fn: Listener) {
  useEffect(() => {
    connect();
    listeners.add(fn);
    return () => {
      listeners.delete(fn);
    };
  }, [fn]);
}

export function useLiveConnection() {
  useEffect(connect, []);
  return useSyncExternalStore(
    (cb) => {
      statusListeners.add(cb);
      return () => statusListeners.delete(cb);
    },
    () => connected,
  );
}
