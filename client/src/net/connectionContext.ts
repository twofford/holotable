// The signed-in layout owns the one WebSocket connection; pages reach it
// through this context instead of opening their own.

import { createContext, use } from "react";
import type { Connection } from "./connection";

export const ConnectionContext = createContext<Connection | null>(null);

export function useConnectionContext(): Connection {
  const connection = use(ConnectionContext);
  if (!connection) throw new Error("useConnectionContext must be used inside the signed-in layout");
  return connection;
}
