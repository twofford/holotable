import { createContext, useContext } from "react";
import type { UserInfo } from "../protocol/generated/UserInfo";

/**
 * The signed-in user, available to any component under `SignedInApp` without
 * passing it down through every level of props.
 */
export const CurrentUserContext = createContext<UserInfo | null>(null);

export function useCurrentUser(): UserInfo {
  const user = useContext(CurrentUserContext);
  if (!user) throw new Error("useCurrentUser must be used inside SignedInApp");
  return user;
}
