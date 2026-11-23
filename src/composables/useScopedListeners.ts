import { onScopeDispose } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";

/** Also release listeners whose registrations finish after unmount. */
export function useScopedListeners() {
  const listeners: UnlistenFn[] = [];
  let disposed = false;

  onScopeDispose(() => {
    disposed = true;
    for (const unlisten of listeners.splice(0)) void unlisten();
  });

  return async (registration: Promise<UnlistenFn>): Promise<void> => {
    const unlisten = await registration;
    if (disposed) void unlisten();
    else listeners.push(unlisten);
  };
}
