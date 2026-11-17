// Typed calls to the server's HTTP API.
//
// Every request sends the session cookie automatically: `credentials:
// "same-origin"` tells fetch to include cookies for our own server. The cookie
// is HttpOnly, so this code never sees the token itself.

import type { ApiError } from "../protocol/generated/ApiError";
import type { ApiErrorCode } from "../protocol/generated/ApiErrorCode";
import type { ChangePasswordRequest } from "../protocol/generated/ChangePasswordRequest";
import type { SignInRequest } from "../protocol/generated/SignInRequest";
import type { SignUpRequest } from "../protocol/generated/SignUpRequest";
import type { UpdateProfileRequest } from "../protocol/generated/UpdateProfileRequest";
import type { UserInfo } from "../protocol/generated/UserInfo";

/** A request the server answered with an error. `message` is safe to show. */
export class ApiRequestError extends Error {
  constructor(
    readonly status: number,
    readonly code: ApiErrorCode,
    message: string,
  ) {
    super(message);
  }
}

/**
 * Sends a request and returns the parsed JSON response.
 * A `Blob` body (such as a `File` from a file picker) is sent as raw bytes;
 * anything else is sent as JSON.
 */
export async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  let headers: Record<string, string> = {};
  let payload: BodyInit | undefined;
  if (body instanceof Blob) {
    headers = { "Content-Type": "application/octet-stream" };
    payload = body;
  } else if (body !== undefined) {
    headers = { "Content-Type": "application/json" };
    payload = JSON.stringify(body);
  }

  let res: Response;
  try {
    res = await fetch(path, { method, credentials: "same-origin", headers, body: payload });
  } catch {
    throw new ApiRequestError(0, "internal", "Can't reach the server. Is it running?");
  }

  // 204 No Content has no body to parse.
  const data: unknown = res.status === 204 ? undefined : await res.json().catch(() => undefined);
  if (!res.ok) {
    const err = data as Partial<ApiError> | undefined;
    throw new ApiRequestError(res.status, err?.error ?? "internal", err?.message ?? `Request failed (${res.status}).`);
  }
  return data as T;
}

export const api = {
  /** The signed-in user, or `null` if there's no valid session. */
  async me(): Promise<UserInfo | null> {
    try {
      return await request<UserInfo>("GET", "/api/auth/me");
    } catch (e) {
      if (e instanceof ApiRequestError && e.status === 401) return null;
      throw e;
    }
  },
  signUp: (req: SignUpRequest) => request<UserInfo>("POST", "/api/auth/signup", req),
  signIn: (req: SignInRequest) => request<UserInfo>("POST", "/api/auth/signin", req),
  signOut: () => request<void>("POST", "/api/auth/signout"),
  updateProfile: (req: UpdateProfileRequest) => request<UserInfo>("PATCH", "/api/account", req),
  changePassword: (req: ChangePasswordRequest) => request<void>("POST", "/api/account/password", req),
};

/** A message to show for any error thrown by the calls above. */
export function errorMessage(e: unknown): string {
  return e instanceof ApiRequestError ? e.message : "Something went wrong. Please try again.";
}
