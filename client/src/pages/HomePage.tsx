import { type FormEvent, useState } from "react";
import { Link, useNavigate } from "react-router";
import { campaigns } from "../api/campaigns";
import { errorMessage } from "../api/client";
import { useLoad } from "../hooks/useLoad";

/** The list of my campaigns, and a form to start a new one (CMP-001, CMP-002). */
export function HomePage() {
  const [list] = useLoad(() => campaigns.list(), []);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const navigate = useNavigate();

  async function create(e: FormEvent) {
    e.preventDefault();
    setError(null);
    try {
      const created = await campaigns.create(name);
      navigate(`/campaigns/${created.id}`);
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  return (
    <div className="page">
      <h2>Your campaigns</h2>
      {list.kind === "loading" && <p className="muted">Loading…</p>}
      {list.kind === "error" && <p className="form-error">{list.message}</p>}
      {list.kind === "ready" &&
        (list.data.length === 0 ? (
          <p className="muted">You're not in any campaigns yet. Start one below, or open an invite link from your GM.</p>
        ) : (
          <ul className="cards">
            {list.data.map((c) => (
              <li key={c.id}>
                <Link to={`/campaigns/${c.id}`} className="card">
                  <strong>{c.name}</strong>
                  <span className={`badge role-${c.role}`}>{c.role === "gm" ? "GM" : "Player"}</span>
                </Link>
              </li>
            ))}
          </ul>
        ))}

      <section className="panel-section">
        <h3>Start a campaign</h3>
        <form className="inline-form" onSubmit={create}>
          <input required maxLength={80} placeholder="Campaign name" value={name} onChange={(e) => setName(e.target.value)} />
          <button type="submit">Create</button>
        </form>
        {error && <p className="form-error">{error}</p>}
      </section>

      <p className="muted">
        <Link to="/sandbox">Map sandbox</Link>: open a map file from this device without uploading it.
      </p>
    </div>
  );
}
