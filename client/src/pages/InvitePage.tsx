import { useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { invites } from "../api/campaigns";
import { errorMessage } from "../api/client";
import { useLoad } from "../hooks/useLoad";

/** `/invite/<token>`: see what the invite is for, then join (CMP-005, CMP-006). */
export function InvitePage() {
  // `useParams` reads the `:token` part of the URL.
  const { token = "" } = useParams();
  const [preview] = useLoad(() => invites.preview(token), [token]);
  const [error, setError] = useState<string | null>(null);
  const navigate = useNavigate();

  async function join() {
    try {
      const campaign = await invites.accept(token);
      navigate(`/campaigns/${campaign.id}`, { replace: true });
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  return (
    <div className="page narrow">
      {preview.kind === "loading" && <p className="muted">Checking your invite…</p>}
      {preview.kind === "error" && (
        <>
          <h2>Invite</h2>
          <p className="form-error">{preview.message}</p>
          <Link to="/">Go to your campaigns</Link>
        </>
      )}
      {preview.kind === "ready" && (
        <>
          <h2>{preview.data.campaignName}</h2>
          {preview.data.alreadyMember ? (
            <>
              <p>You're already in this campaign.</p>
              <Link className="button" to={`/campaigns/${preview.data.campaignId}`}>
                Open it
              </Link>
            </>
          ) : (
            <>
              <p>You've been invited to join as {preview.data.role === "gm" ? "a GM" : "a player"}.</p>
              <button type="button" onClick={join}>
                Join campaign
              </button>
            </>
          )}
          {error && <p className="form-error">{error}</p>}
        </>
      )}
    </div>
  );
}
