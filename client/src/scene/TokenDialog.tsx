import { useEffect, useState } from "react";
import { campaigns } from "../api/campaigns";
import type { AssetInfo } from "../protocol/generated/AssetInfo";
import type { MemberInfo } from "../protocol/generated/MemberInfo";
import type { TokenSettings } from "../protocol/generated/TokenSettings";

interface Props {
  campaignId: number;
  /** The token being edited; `null` for a new one. */
  initial: TokenSettings | null;
  members: readonly MemberInfo[];
  onSave: (settings: TokenSettings) => void;
  onClose: () => void;
}

/** GM form for a token's name, picture, size, owner and visibility (TOK-001, TOK-004). */
export function TokenDialog({ campaignId, initial, members, onSave, onClose }: Props) {
  const [name, setName] = useState(initial?.name ?? "");
  const [assetId, setAssetId] = useState<number | null>(initial?.assetId ?? null);
  const [size, setSize] = useState(initial?.size ?? 1);
  const [ownerId, setOwnerId] = useState<number | null>(initial?.ownerId ?? null);
  const [hidden, setHidden] = useState(initial?.hidden ?? false);
  const [images, setImages] = useState<AssetInfo[]>([]);

  useEffect(() => {
    campaigns
      .assets(campaignId)
      .then((all) => setImages(all.filter((a) => a.kind === "image")))
      .catch(() => setImages([]));
  }, [campaignId]);

  // Escape closes, like any dialog.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const players = members.filter((m) => m.role === "player");

  return (
    <div className="modal-backdrop" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <form
        className="modal"
        aria-label={initial ? "Edit token" : "New token"}
        onSubmit={(e) => {
          e.preventDefault();
          onSave({ name: name.trim(), assetId, size, ownerId, hidden });
        }}
      >
        <h3>{initial ? "Edit token" : "New token"}</h3>
        <label>
          Name
          <input required autoFocus maxLength={40} value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label>
          Picture
          <select value={assetId ?? ""} onChange={(e) => setAssetId(e.target.value ? Number(e.target.value) : null)}>
            <option value="">None (initial on a disc)</option>
            {images.map((a) => (
              <option key={a.id} value={a.id}>
                {a.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Size
          <select value={size} onChange={(e) => setSize(Number(e.target.value))}>
            {[1, 2, 3, 4].map((n) => (
              <option key={n} value={n}>
                {n}×{n} squares
              </option>
            ))}
          </select>
        </label>
        <label>
          Owner
          <select value={ownerId ?? ""} onChange={(e) => setOwnerId(e.target.value ? Number(e.target.value) : null)}>
            <option value="">GM only</option>
            {players.map((m) => (
              <option key={m.userId} value={m.userId}>
                {m.displayName}
              </option>
            ))}
          </select>
        </label>
        <label className="check">
          <input type="checkbox" checked={hidden} onChange={(e) => setHidden(e.target.checked)} />
          Hidden from players
        </label>
        <div className="row-actions">
          <button type="submit" disabled={!name.trim()}>
            {initial ? "Save" : "Add token"}
          </button>
          <button type="button" className="secondary" onClick={onClose}>
            Cancel
          </button>
        </div>
      </form>
    </div>
  );
}
