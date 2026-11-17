import { useEffect, useState, useSyncExternalStore } from "react";
import { BrowserRouter, Link, Outlet, Route, Routes, useLocation } from "react-router";
import { api } from "./api/client";
import { AccountMenu } from "./auth/AccountMenu";
import { AuthScreen } from "./auth/AuthScreen";
import { CurrentUserContext } from "./auth/currentUser";
import { ConnectionContext } from "./net/connectionContext";
import { Connection } from "./net/connection";
import { CampaignPage } from "./pages/CampaignPage";
import { HomePage } from "./pages/HomePage";
import { InvitePage } from "./pages/InvitePage";
import { NotFoundPage } from "./pages/NotFoundPage";
import { SandboxPage } from "./pages/SandboxPage";
import { ScenePage } from "./pages/ScenePage";
import type { UserInfo } from "./protocol/generated/UserInfo";

/** Where the app is with sign-in. A union type makes each case explicit. */
type Session = { kind: "checking" } | { kind: "signedOut" } | { kind: "signedIn"; user: UserInfo };

export function App() {
  return (
    // The router reads the address bar. The server sends index.html for any
    // path it doesn't know, so /campaigns/3 loads the app, which shows the page.
    <BrowserRouter>
      <SessionGate />
    </BrowserRouter>
  );
}

/** Shows sign-in until there's a session, then the app. */
function SessionGate() {
  const [session, setSession] = useState<Session>({ kind: "checking" });

  // On first load, ask the server whether the browser already has a session.
  useEffect(() => {
    api
      .me()
      .then((user) => setSession(user ? { kind: "signedIn", user } : { kind: "signedOut" }))
      .catch(() => setSession({ kind: "signedOut" }));
  }, []);

  switch (session.kind) {
    case "checking":
      return <div className="splash">Holotable</div>;
    case "signedOut":
      // The address bar keeps its path (an invite link, say) while signing in,
      // so the right page shows afterwards (CMP-005).
      return <AuthScreen onSignedIn={(user) => setSession({ kind: "signedIn", user })} />;
    case "signedIn":
      return (
        <CurrentUserContext value={session.user}>
          <Routes>
            <Route
              element={
                <SignedInLayout
                  user={session.user}
                  onUserChange={(user) => setSession({ kind: "signedIn", user })}
                  onSignedOut={() => setSession({ kind: "signedOut" })}
                />
              }
            >
              <Route index element={<HomePage />} />
              <Route path="campaigns/:campaignId" element={<CampaignPage />} />
              <Route path="campaigns/:campaignId/scenes/:sceneId" element={<ScenePage />} />
              <Route path="invite/:token" element={<InvitePage />} />
              <Route path="sandbox" element={<SandboxPage />} />
              <Route path="*" element={<NotFoundPage />} />
            </Route>
          </Routes>
        </CurrentUserContext>
      );
  }
}

interface LayoutProps {
  user: UserInfo;
  onUserChange: (user: UserInfo) => void;
  onSignedOut: () => void;
}

/**
 * The header around every signed-in page; `<Outlet />` is where the page goes.
 * The WebSocket connection lives here, so it opens on sign-in and closes on
 * sign-out, when this unmounts.
 */
function SignedInLayout({ user, onUserChange, onSignedOut }: LayoutProps) {
  const { connection, state } = useConnection();
  const location = useLocation();

  // If the socket drops, the session may have ended (expired, or the password
  // was changed on another device). Check, and go back to sign-in if so.
  useEffect(() => {
    if (state.status !== "closed") return;
    api
      .me()
      .then((me) => {
        if (!me) onSignedOut();
      })
      .catch(() => undefined); // Server unreachable: keep trying to reconnect.
  }, [state.status, onSignedOut]);

  return (
    <main className="app">
      <header>
        <h1>
          <Link to="/">Holotable</Link>
        </h1>
        <span className="spacer" />
        {/* `key` closes the menu when the page changes. */}
        <details className="menu" key={`conn-${location.pathname}`}>
          <summary className="menu-button">
            <span className={`status status-${state.status}`}>{state.status}</span>
          </summary>
          <div className="panel">
            <p>
              Server version: <strong>{state.serverVersion ?? "—"}</strong>
              <br />
              Last round trip: <strong>{state.lastRttMs === null ? "—" : `${state.lastRttMs} ms`}</strong>
            </p>
            <button type="button" onClick={() => connection.sendPing()} disabled={state.status !== "open"}>
              Ping server
            </button>
            <ol className="log">
              {state.log.map((entry) => (
                <li key={entry.id} className={`log-${entry.direction}`}>
                  <span className="arrow">{entry.direction === "in" ? "←" : "→"}</span>
                  <code>{entry.text}</code>
                </li>
              ))}
            </ol>
          </div>
        </details>
        <AccountMenu key={`acct-${location.pathname}`} user={user} onUserChange={onUserChange} onSignedOut={onSignedOut} />
      </header>
      <ConnectionContext value={connection}>
        <Outlet />
      </ConnectionContext>
    </main>
  );
}

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
