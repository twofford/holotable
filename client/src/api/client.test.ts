import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiRequestError, api, errorMessage } from "./client";

/** Replaces the global fetch with one that always returns `response`. */
function mockFetch(status: number, body?: unknown) {
  const fn = vi.fn(async () => new Response(body === undefined ? null : JSON.stringify(body), { status }));
  vi.stubGlobal("fetch", fn);
  return fn;
}

afterEach(() => vi.unstubAllGlobals());

describe("api client", () => {
  it("me() returns null when not signed in", async () => {
    mockFetch(401, { error: "unauthorized", message: "You're not signed in." });
    expect(await api.me()).toBeNull();
  });

  it("sends JSON with the session cookie", async () => {
    const fetch = mockFetch(200, { id: 1, email: "a@b.co", displayName: "A" });
    await api.signIn({ email: "a@b.co", password: "x" });
    expect(fetch).toHaveBeenCalledWith(
      "/api/auth/signin",
      expect.objectContaining({ method: "POST", credentials: "same-origin", body: '{"email":"a@b.co","password":"x"}' }),
    );
  });

  it("turns error responses into ApiRequestError", async () => {
    mockFetch(409, { error: "email_taken", message: "An account with this email already exists." });
    const err = await api.signUp({ email: "a@b.co", password: "0123456789", displayName: "A" }).catch((e) => e);
    expect(err).toBeInstanceOf(ApiRequestError);
    expect(err.code).toBe("email_taken");
    expect(errorMessage(err)).toBe("An account with this email already exists.");
  });

  it("handles 204 No Content", async () => {
    mockFetch(204);
    await expect(api.signOut()).resolves.toBeUndefined();
  });

  it("reports an unreachable server", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => Promise.reject(new TypeError("Failed to fetch"))));
    expect(errorMessage(await api.me().catch((e) => e))).toMatch(/server/);
  });
});
