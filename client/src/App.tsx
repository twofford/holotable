import { useEffect, useState, useSyncExternalStore } from "react";
import { Connection } from "./net/connection";

/** Creates one Connection for the component's lifetime and subscribes to its state. */
function useConnection() {
  // The initializer function runs once, so we keep the same Connection across renders.
  const [connection] = useState(() => Connection.forCurrentPage());

  useEffect(() => {
    connection.start();
    return () => connection.stop();
  }, [connection]);

  const state = useSyncExternalStore(connection.subscribe, connection.getSnapshot);
  return { connection, state };
}

export function App() {
  const { connection, state } = useConnection();

  return (
    <main className="app">
      <header>
        <h1>Holotable</h1>
        <span className={`status status-${state.status}`}>{state.status}</span>
      </header>

      <section className="panel">
        <p>
          Server version: <strong>{state.serverVersion ?? "—"}</strong>
          <br />
          Last round trip: <strong>{state.lastRttMs === null ? "—" : `${state.lastRttMs} ms`}</strong>
        </p>
        <button type="button" onClick={() => connection.sendPing()} disabled={state.status !== "open"}>
          Ping server
        </button>
        <p className="hint">The server also pings this page every 5 seconds, and the page answers.</p>
      </section>

      <section className="panel">
        <h2>Messages</h2>
        <ol className="log">
          {state.log.map((entry) => (
            <li key={entry.id} className={`log-${entry.direction}`}>
              <span className="arrow">{entry.direction === "in" ? "←" : "→"}</span>
              <code>{entry.text}</code>
            </li>
          ))}
        </ol>
      </section>
    </main>
  );
}
