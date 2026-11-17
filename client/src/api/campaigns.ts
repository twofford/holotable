// Typed calls for campaigns, members, invites, uploads and scenes.

import type { AssetInfo } from "../protocol/generated/AssetInfo";
import type { CampaignDetail } from "../protocol/generated/CampaignDetail";
import type { CampaignSummary } from "../protocol/generated/CampaignSummary";
import type { CreateInviteRequest } from "../protocol/generated/CreateInviteRequest";
import type { CreatedInvite } from "../protocol/generated/CreatedInvite";
import type { InviteInfo } from "../protocol/generated/InviteInfo";
import type { InvitePreview } from "../protocol/generated/InvitePreview";
import type { MemberInfo } from "../protocol/generated/MemberInfo";
import type { Role } from "../protocol/generated/Role";
import type { SceneDetail } from "../protocol/generated/SceneDetail";
import type { SceneSummary } from "../protocol/generated/SceneSummary";
import type { UpdateSceneRequest } from "../protocol/generated/UpdateSceneRequest";
import { request } from "./client";

const base = (campaignId: number) => `/api/campaigns/${campaignId}`;

export const campaigns = {
  list: () => request<CampaignSummary[]>("GET", "/api/campaigns"),
  create: (name: string) => request<CampaignSummary>("POST", "/api/campaigns", { name }),
  get: (id: number) => request<CampaignDetail>("GET", base(id)),
  rename: (id: number, name: string) => request<CampaignSummary>("PATCH", base(id), { name }),
  remove: (id: number, confirmName: string) => request<void>("DELETE", base(id), { confirmName }),
  leave: (id: number) => request<void>("POST", `${base(id)}/leave`),

  changeRole: (id: number, userId: number, role: Role) =>
    request<MemberInfo>("PATCH", `${base(id)}/members/${userId}`, { role }),
  removeMember: (id: number, userId: number) => request<void>("DELETE", `${base(id)}/members/${userId}`),

  invites: (id: number) => request<InviteInfo[]>("GET", `${base(id)}/invites`),
  createInvite: (id: number, req: CreateInviteRequest) => request<CreatedInvite>("POST", `${base(id)}/invites`, req),
  revokeInvite: (id: number, inviteId: number) => request<void>("DELETE", `${base(id)}/invites/${inviteId}`),

  assets: (id: number) => request<AssetInfo[]>("GET", `${base(id)}/assets`),
  /** Uploads a file as-is. `encodeURIComponent` keeps odd file names from breaking the URL. */
  upload: (id: number, file: File) =>
    request<AssetInfo>("POST", `${base(id)}/assets?name=${encodeURIComponent(file.name)}`, file),
  removeAsset: (id: number, assetId: number) => request<void>("DELETE", `${base(id)}/assets/${assetId}`),
  tileUrl: (id: number, assetId: number, index: number) => `${base(id)}/assets/${assetId}/tiles/${index}`,

  createScene: (id: number, name: string, assetId: number) =>
    request<SceneSummary>("POST", `${base(id)}/scenes`, { name, assetId }),
  scene: (id: number, sceneId: number) => request<SceneDetail>("GET", `${base(id)}/scenes/${sceneId}`),
  updateScene: (id: number, sceneId: number, req: UpdateSceneRequest) =>
    request<SceneDetail>("PATCH", `${base(id)}/scenes/${sceneId}`, req),
  removeScene: (id: number, sceneId: number) => request<void>("DELETE", `${base(id)}/scenes/${sceneId}`),
  /** Puts everyone on this scene; `null` clears it (MAP-010). */
  setActiveScene: (id: number, sceneId: number | null) =>
    request<void>("PUT", `${base(id)}/active-scene`, { sceneId }),
};

export const invites = {
  preview: (token: string) => request<InvitePreview>("GET", `/api/invites/${encodeURIComponent(token)}`),
  accept: (token: string) => request<CampaignSummary>("POST", `/api/invites/${encodeURIComponent(token)}/accept`),
};
