import { type FormEvent, useEffect, useReducer, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { campaigns } from "../api/campaigns";
import { errorMessage } from "../api/client";
import { useCurrentUser } from "../auth/currentUser";
import { useLoad } from "../hooks/useLoad";
import { useConnectionContext } from "../net/connectionContext";
import type { AssetInfo } from "../protocol/generated/AssetInfo";
import type { CampaignDetail } from "../protocol/generated/CampaignDetail";
import type { CreatedInvite } from "../protocol/generated/CreatedInvite";
import type { Role } from "../protocol/generated/Role";

/**
 * One campaign: scenes, members, and for GMs, uploads, invites and settings.
 * The server checks every action; the page only hides what a player can't do.
 */
export function CampaignPage() {
  const campaignId = Number(useParams().campaignId);
  const [campaign, reload] = useLoad(() => campaigns.get(campaignId), [campaignId]);
  // Uploads are listed in two places (the uploads list, and the map picker for
  // new scenes). Both reload when this number goes up after any change.
  const [assetsVersion, assetsChanged] = useReducer((n: number) => n + 1, 0);
  const connection = useConnectionContext();
  const navigate = useNavigate();

  // When the GM puts everyone on a scene, players go straight there (MAP-010);
  // the GM's own page just shows the new "Now playing" badge.
  const isGmRef = useRef(false);
  isGmRef.current = campaign.kind === "ready" && campaign.data.role === "gm";
  useEffect(
    () =>
      connection.onMessage((msg) => {
        if (msg.type !== "activeScene" || msg.campaignId !== campaignId) return;
        if (isGmRef.current) reload();
        else navigate(`/campaigns/${campaignId}/scenes/${msg.sceneId}`);
      }),
    [connection, campaignId, navigate, reload],
  );

  if (campaign.kind === "loading") return <div className="page muted">Loading…</div>;
  if (campaign.kind === "error") {
    return (
      <div className="page">
        <p className="form-error">{campaign.message}</p>
        <Link to="/">Back to your campaigns</Link>
      </div>
    );
  }

  const c = campaign.data;
  const isGm = c.role === "gm";
  return (
    <div className="page">
      <Link to="/" className="muted">
        ← Campaigns
      </Link>
      <h2>
        {c.name} <span className={`badge role-${c.role}`}>{isGm ? "GM" : "Player"}</span>
      </h2>
      <ScenesSection campaign={c} reload={reload} assetsVersion={assetsVersion} onScenesChanged={assetsChanged} />
      {isGm && <MapsSection campaignId={c.id} assetsVersion={assetsVersion} onChange={assetsChanged} />}
      <MembersSection campaign={c} reload={reload} />
      {isGm && <InvitesSection campaignId={c.id} />}
      <SettingsSection campaign={c} reload={reload} />
    </div>
  );
}

interface SectionProps {
  campaign: CampaignDetail;
  reload: () => void;
}

interface ScenesProps extends SectionProps {
  assetsVersion: number;
  /** Scenes changed, so the uploads' "used by" lists did too. */
  onScenesChanged: () => void;
}

function ScenesSection({ campaign, reload, assetsVersion, onScenesChanged }: ScenesProps) {
  const isGm = campaign.role === "gm";
  return (
    <section className="panel-section">
      <h3>Scenes</h3>
      {campaign.scenes.length === 0 ? (
        <p className="muted">{isGm ? "No scenes yet. Upload a map below, then make a scene from it." : "No scenes yet."}</p>
      ) : (
        <ul className="rows">
          {campaign.scenes.map((s) => (
            <li key={s.id}>
              <span className="row-actions">
                <Link to={`/campaigns/${campaign.id}/scenes/${s.id}`}>{s.name}</Link>
                {campaign.activeSceneId === s.id && <span className="badge now-playing">Now playing</span>}
              </span>
              {isGm && (
                <span className="row-actions">
                  {campaign.activeSceneId !== s.id && (
                    <ActionButton
                      label="Play for everyone"
                      action={() => campaigns.setActiveScene(campaign.id, s.id)}
                      onDone={reload}
                    />
                  )}
                  <ActionButton
                    label="Delete"
                    confirm={`Delete the scene "${s.name}"? The map upload stays.`}
                    action={() => campaigns.removeScene(campaign.id, s.id)}
                    onDone={() => {
                      reload();
                      onScenesChanged();
                    }}
                  />
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
      {isGm && <NewSceneForm campaignId={campaign.id} assetsVersion={assetsVersion} onCreated={reload} />}
    </section>
  );
}

interface NewSceneProps {
  campaignId: number;
  assetsVersion: number;
  onCreated: () => void;
}

function NewSceneForm({ campaignId, assetsVersion, onCreated }: NewSceneProps) {
  const [assets] = useLoad(() => campaigns.assets(campaignId), [campaignId, assetsVersion]);
  const [name, setName] = useState("");
  const [assetId, setAssetId] = useState("");
  const [error, setError] = useState<string | null>(null);
  const navigate = useNavigate();

  const maps = assets.kind === "ready" ? assets.data.filter((a) => a.kind === "uvtt") : [];
  if (maps.length === 0) return null;

  async function submit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    try {
      const scene = await campaigns.createScene(campaignId, name, Number(assetId || maps[0]!.id));
      onCreated();
      navigate(`/campaigns/${campaignId}/scenes/${scene.id}`);
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  return (
    <form className="inline-form" onSubmit={submit}>
      <input required maxLength={80} placeholder="New scene name" value={name} onChange={(e) => setName(e.target.value)} />
      <select value={assetId} onChange={(e) => setAssetId(e.target.value)} aria-label="Map">
        {maps.map((m) => (
          <option key={m.id} value={m.id}>
            {m.name}
          </option>
        ))}
      </select>
      <button type="submit">Create scene</button>
      {error && <p className="form-error">{error}</p>}
    </form>
  );
}

interface MapsProps {
  campaignId: number;
  assetsVersion: number;
  /** Call after an upload or delete, so every list of uploads refreshes. */
  onChange: () => void;
}

function MapsSection({ campaignId, assetsVersion, onChange }: MapsProps) {
  const [assets] = useLoad(() => campaigns.assets(campaignId), [campaignId, assetsVersion]);
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function uploadFiles(files: FileList) {
    setBusy(true);
    setStatus(null);
    try {
      // One at a time, so a big map doesn't compete with the others.
      for (const file of Array.from(files)) {
        setStatus(`Uploading ${file.name}…`);
        await campaigns.upload(campaignId, file);
      }
      setStatus("Uploaded.");
      onChange();
    } catch (err) {
      setStatus(errorMessage(err));
      onChange(); // Earlier files in the batch may have uploaded.
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="panel-section">
      <h3>Maps and images</h3>
      <label className={`button secondary ${busy ? "disabled" : ""}`}>
        Upload…
        <input
          type="file"
          multiple
          disabled={busy}
          accept=".uvtt,.dd2vtt,.png,.jpg,.jpeg,.webp"
          onChange={(e) => {
            if (e.target.files?.length) uploadFiles(e.target.files);
            e.target.value = "";
          }}
        />
      </label>
      <span className="hint"> Maps (.uvtt, .dd2vtt) up to 50 MB; images up to 20 MB.</span>
      {status && <p className="hint">{status}</p>}
      {assets.kind === "ready" && assets.data.length > 0 && (
        <ul className="rows">
          {assets.data.map((a) => (
            <AssetRow key={a.id} campaignId={campaignId} asset={a} onDeleted={onChange} />
          ))}
        </ul>
      )}
    </section>
  );
}

function AssetRow({ campaignId, asset, onDeleted }: { campaignId: number; asset: AssetInfo; onDeleted: () => void }) {
  return (
    <li>
      <span>
        {asset.name}{" "}
        <span className="muted">
          · {asset.kind === "uvtt" ? "map" : "image"} · {asset.width}×{asset.height} px · {formatBytes(asset.sizeBytes)}
          {asset.usedBy.length > 0 && <> · used by {asset.usedBy.join(", ")}</>}
        </span>
      </span>
      <ActionButton
        label="Delete"
        confirm={`Delete ${asset.name}?`}
        action={() => campaigns.removeAsset(campaignId, asset.id)}
        onDone={onDeleted}
      />
    </li>
  );
}

/** 11341 → "11 KB", 1147680 → "1.1 MB". */
export function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function MembersSection({ campaign, reload }: SectionProps) {
  const me = useCurrentUser();
  const isGm = campaign.role === "gm";
  const [error, setError] = useState<string | null>(null);

  async function change(userId: number, role: Role) {
    setError(null);
    try {
      await campaigns.changeRole(campaign.id, userId, role);
      reload();
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  return (
    <section className="panel-section">
      <h3>Members</h3>
      <ul className="rows">
        {campaign.members.map((m) => (
          <li key={m.userId}>
            <span>
              {m.displayName}
              {m.userId === me.id && <span className="muted"> (you)</span>}
            </span>
            {isGm ? (
              <span className="row-actions">
                <select
                  value={m.role}
                  aria-label={`Role for ${m.displayName}`}
                  onChange={(e) => change(m.userId, e.target.value as Role)}
                >
                  <option value="gm">GM</option>
                  <option value="player">Player</option>
                </select>
                {m.userId !== me.id && (
                  <ActionButton
                    label="Remove"
                    confirm={`Remove ${m.displayName} from ${campaign.name}?`}
                    action={() => campaigns.removeMember(campaign.id, m.userId)}
                    onDone={reload}
                    onError={setError}
                  />
                )}
              </span>
            ) : (
              <span className={`badge role-${m.role}`}>{m.role === "gm" ? "GM" : "Player"}</span>
            )}
          </li>
        ))}
      </ul>
      {error && <p className="form-error">{error}</p>}
    </section>
  );
}

function InvitesSection({ campaignId }: { campaignId: number }) {
  const [list, reloadList] = useLoad(() => campaigns.invites(campaignId), [campaignId]);
  const [role, setRole] = useState<Role>("player");
  const [days, setDays] = useState(7);
  const [uses, setUses] = useState(1);
  const [created, setCreated] = useState<CreatedInvite | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function create(e: FormEvent) {
    e.preventDefault();
    setError(null);
    try {
      setCreated(await campaigns.createInvite(campaignId, { role, expiresInDays: days, maxUses: uses }));
      reloadList();
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  const link = created ? `${location.origin}/invite/${created.token}` : "";
  return (
    <section className="panel-section">
      <h3>Invites</h3>
      <form className="inline-form" onSubmit={create}>
        <label>
          Role
          <select value={role} onChange={(e) => setRole(e.target.value as Role)}>
            <option value="player">Player</option>
            <option value="gm">GM</option>
          </select>
        </label>
        <label>
          Days
          <input type="number" min={1} max={30} value={days} onChange={(e) => setDays(Number(e.target.value))} />
        </label>
        <label>
          Uses
          <input type="number" min={1} max={100} value={uses} onChange={(e) => setUses(Number(e.target.value))} />
        </label>
        <button type="submit">Create invite link</button>
      </form>
      {error && <p className="form-error">{error}</p>}
      {created && (
        <div className="invite-link">
          <p className="hint">Send this link to your player. It's shown only once.</p>
          <input readOnly value={link} onFocus={(e) => e.target.select()} aria-label="Invite link" />
          <button type="button" className="secondary" onClick={() => navigator.clipboard?.writeText(link)}>
            Copy
          </button>
        </div>
      )}
      {list.kind === "ready" && list.data.length > 0 && (
        <ul className="rows">
          {list.data.map((i) => (
            <li key={i.id}>
              <span>
                {i.role === "gm" ? "GM" : "Player"} invite{" "}
                <span className="muted">
                  · used {i.uses} of {i.maxUses} · expires {new Date(i.expiresAt * 1000).toLocaleDateString()}
                </span>
              </span>
              <ActionButton label="Revoke" action={() => campaigns.revokeInvite(campaignId, i.id)} onDone={reloadList} />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function SettingsSection({ campaign, reload }: SectionProps) {
  const isGm = campaign.role === "gm";
  const navigate = useNavigate();
  const [name, setName] = useState(campaign.name);
  const [confirmName, setConfirmName] = useState("");
  const [error, setError] = useState<string | null>(null);

  async function run(action: () => Promise<unknown>, after: () => void) {
    setError(null);
    try {
      await action();
      after();
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  return (
    <section className="panel-section">
      <h3>Settings</h3>
      {isGm && (
        <form
          className="inline-form"
          onSubmit={(e) => {
            e.preventDefault();
            run(() => campaigns.rename(campaign.id, name), reload);
          }}
        >
          <input required maxLength={80} value={name} onChange={(e) => setName(e.target.value)} aria-label="Campaign name" />
          <button type="submit" className="secondary" disabled={name.trim() === campaign.name}>
            Rename
          </button>
        </form>
      )}
      <button
        type="button"
        className="secondary"
        onClick={() => confirm(`Leave ${campaign.name}?`) && run(() => campaigns.leave(campaign.id), () => navigate("/"))}
      >
        Leave campaign
      </button>
      {isGm && (
        <form
          className="inline-form danger"
          onSubmit={(e) => {
            e.preventDefault();
            run(() => campaigns.remove(campaign.id, confirmName), () => navigate("/"));
          }}
        >
          <input
            placeholder={`Type "${campaign.name}" to delete`}
            value={confirmName}
            onChange={(e) => setConfirmName(e.target.value)}
            aria-label="Confirm campaign name"
          />
          <button type="submit" className="danger" disabled={confirmName.trim() !== campaign.name}>
            Delete campaign
          </button>
        </form>
      )}
      {error && <p className="form-error">{error}</p>}
    </section>
  );
}

interface ActionButtonProps {
  label: string;
  /** If set, ask before doing it. */
  confirm?: string;
  action: () => Promise<unknown>;
  onDone: () => void;
  onError?: (message: string) => void;
}

/** A small button that runs a server action and reports errors. */
function ActionButton({ label, confirm: question, action, onDone, onError }: ActionButtonProps) {
  const [busy, setBusy] = useState(false);
  return (
    <button
      type="button"
      className="secondary small"
      disabled={busy}
      onClick={async () => {
        if (question && !window.confirm(question)) return;
        setBusy(true);
        try {
          await action();
          onDone();
        } catch (err) {
          (onError ?? window.alert)(errorMessage(err));
        } finally {
          setBusy(false);
        }
      }}
    >
      {label}
    </button>
  );
}
