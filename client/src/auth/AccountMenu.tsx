import { type FormEvent, useState } from "react";
import { api, errorMessage } from "../api/client";
import type { UserInfo } from "../protocol/generated/UserInfo";

interface Props {
  user: UserInfo;
  onUserChange: (user: UserInfo) => void;
  onSignedOut: () => void;
}

/** The header's account menu: display name, password, sign out. */
export function AccountMenu({ user, onUserChange, onSignedOut }: Props) {
  return (
    <details className="menu">
      <summary className="menu-button">{user.displayName}</summary>
      <div className="panel account">
        <p className="muted">{user.email}</p>
        <NameForm user={user} onUserChange={onUserChange} />
        <PasswordForm />
        <button
          type="button"
          className="secondary"
          onClick={async () => {
            // Sign out locally even if the request fails: the user asked to leave.
            await api.signOut().catch(() => undefined);
            onSignedOut();
          }}
        >
          Sign out
        </button>
      </div>
    </details>
  );
}

function NameForm({ user, onUserChange }: Pick<Props, "user" | "onUserChange">) {
  const [name, setName] = useState(user.displayName);
  const [status, setStatus] = useState<string | null>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    try {
      onUserChange(await api.updateProfile({ displayName: name }));
      setStatus("Saved.");
    } catch (err) {
      setStatus(errorMessage(err));
    }
  }

  return (
    <form onSubmit={submit}>
      <label>
        Display name
        <input required maxLength={50} value={name} onChange={(e) => setName(e.target.value)} />
      </label>
      <button type="submit" disabled={name.trim() === user.displayName}>
        Save name
      </button>
      {status && <p className="hint">{status}</p>}
    </form>
  );
}

function PasswordForm() {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [status, setStatus] = useState<string | null>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    try {
      await api.changePassword({ currentPassword: current, newPassword: next });
      setCurrent("");
      setNext("");
      setStatus("Password changed. Your other devices have been signed out.");
    } catch (err) {
      setStatus(errorMessage(err));
    }
  }

  return (
    <form onSubmit={submit}>
      <label>
        Current password
        <input type="password" required autoComplete="current-password" value={current} onChange={(e) => setCurrent(e.target.value)} />
      </label>
      <label>
        New password
        <input type="password" required minLength={10} autoComplete="new-password" value={next} onChange={(e) => setNext(e.target.value)} />
      </label>
      <button type="submit">Change password</button>
      {status && <p className="hint">{status}</p>}
    </form>
  );
}
