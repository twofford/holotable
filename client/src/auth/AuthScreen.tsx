import { type FormEvent, useState } from "react";
import { api, errorMessage } from "../api/client";
import type { UserInfo } from "../protocol/generated/UserInfo";

/** Same minimum as the server (holotable-core `MIN_PASSWORD_CHARS`). */
const MIN_PASSWORD_CHARS = 10;

type Mode = "signIn" | "signUp";

/** The sign-in and sign-up forms, shown when there's no session. */
export function AuthScreen({ onSignedIn }: { onSignedIn: (user: UserInfo) => void }) {
  const [mode, setMode] = useState<Mode>("signIn");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const signingUp = mode === "signUp";

  async function submit(e: FormEvent) {
    // A form's default action reloads the page; we send the data with fetch instead.
    e.preventDefault();
    setError(null);
    if (signingUp && [...password].length < MIN_PASSWORD_CHARS) {
      setError(`Use at least ${MIN_PASSWORD_CHARS} characters for your password.`);
      return;
    }
    setBusy(true);
    try {
      const user = signingUp
        ? await api.signUp({ email, password, displayName })
        : await api.signIn({ email, password });
      onSignedIn(user);
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  }

  function switchTo(next: Mode) {
    setMode(next);
    setError(null);
  }

  return (
    <div className="auth-screen">
      <form className="auth-card" onSubmit={submit}>
        <h1>Holotable</h1>
        {location.pathname.startsWith("/invite/") && (
          <p className="hint center">Sign in or create an account to accept your invite.</p>
        )}
        <div className="tabs" role="tablist">
          <button type="button" role="tab" aria-selected={!signingUp} onClick={() => switchTo("signIn")}>
            Sign in
          </button>
          <button type="button" role="tab" aria-selected={signingUp} onClick={() => switchTo("signUp")}>
            Create account
          </button>
        </div>

        <label>
          Email
          {/* `autoComplete` lets password managers and iPad keyboards fill these in. */}
          <input type="email" required autoComplete="email" value={email} onChange={(e) => setEmail(e.target.value)} />
        </label>
        {signingUp && (
          <label>
            Display name
            <input
              required
              maxLength={50}
              autoComplete="nickname"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
            />
          </label>
        )}
        <label>
          Password
          <input
            type="password"
            required
            autoComplete={signingUp ? "new-password" : "current-password"}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
          {signingUp && <span className="hint">At least {MIN_PASSWORD_CHARS} characters.</span>}
        </label>

        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
        <button type="submit" disabled={busy}>
          {busy ? "Please wait…" : signingUp ? "Create account" : "Sign in"}
        </button>
      </form>
    </div>
  );
}
