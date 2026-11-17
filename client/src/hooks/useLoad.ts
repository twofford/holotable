import { useCallback, useEffect, useState } from "react";
import { errorMessage } from "../api/client";

/** The state of something being fetched. A union, so each case is explicit. */
export type Load<T> = { kind: "loading" } | { kind: "error"; message: string } | { kind: "ready"; data: T };

/**
 * Runs `fetcher` when the component mounts and whenever `deps` change, and
 * returns its state plus a `reload` function for after a change.
 *
 * If the inputs change while a fetch is still running, the stale result is
 * ignored (`cancelled`), so a slow old response can't overwrite a newer one.
 */
export function useLoad<T>(fetcher: () => Promise<T>, deps: readonly unknown[]) {
  const [state, setState] = useState<Load<T>>({ kind: "loading" });
  const [version, setVersion] = useState(0);
  const reload = useCallback(() => setVersion((v) => v + 1), []);

  useEffect(() => {
    let cancelled = false;
    fetcher()
      .then((data) => !cancelled && setState({ kind: "ready", data }))
      .catch((e) => !cancelled && setState({ kind: "error", message: errorMessage(e) }));
    return () => {
      cancelled = true;
    };
    // `fetcher` is a new function on every render; `deps` says when to refetch.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, version]);

  return [state, reload] as const;
}
