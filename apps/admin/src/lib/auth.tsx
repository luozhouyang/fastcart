import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { AdminClient } from "@fastcart/admin-sdk";
import { ApiError } from "@fastcart/api-client";
import type { Store, User } from "@fastcart/shared-types";

import {
  DEMO_STORAGE_KEY,
  STORE_STORAGE_KEY,
  createAdmin,
  errorMessage,
  fetchStores,
} from "@/lib/api";
import { demoStore } from "@/lib/store";

export type AuthStatus = "loading" | "anonymous" | "authenticated" | "no-store";
export type AuthMode = "api" | "demo";

export interface AuthUser {
  name: string;
  email: string;
  role: string;
}

interface AuthContextValue {
  status: AuthStatus;
  mode: AuthMode;
  user: AuthUser | null;
  stores: Store[];
  store: Store | null;
  client: AdminClient | null;
  permissions: string[];
  connectionError: string | null;
  dataKey: number;
  login: (email: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
  enterDemo: () => void;
  selectStore: (storeId: string) => void;
  refreshStores: () => Promise<void>;
  can: (permission: string) => boolean;
}

const AuthContext = createContext<AuthContextValue | null>(null);

function displayUser(user: User): AuthUser {
  const name = [user.first_name, user.last_name].filter(Boolean).join(" ").trim();
  return {
    name: name.length > 0 ? name : user.email,
    email: user.email,
    role: "Owner",
  };
}

function readDemoFlag(): boolean {
  try {
    return window.sessionStorage.getItem(DEMO_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function writeDemoFlag(enabled: boolean): void {
  try {
    if (enabled) window.sessionStorage.setItem(DEMO_STORAGE_KEY, "1");
    else window.sessionStorage.removeItem(DEMO_STORAGE_KEY);
  } catch {
    // Session storage may be unavailable; demo mode still works for this load.
  }
}

function readStoreId(): string | null {
  try {
    return window.localStorage.getItem(STORE_STORAGE_KEY);
  } catch {
    return null;
  }
}

function writeStoreId(storeId: string | null): void {
  try {
    if (storeId === null) window.localStorage.removeItem(STORE_STORAGE_KEY);
    else window.localStorage.setItem(STORE_STORAGE_KEY, storeId);
  } catch {
    // Preference persistence is best effort.
  }
}

const DEMO_USER: AuthUser = {
  name: "Zhouyang Luo",
  email: "hello@halcyonsupply.com",
  role: "Owner",
};

export function AuthProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<AuthStatus>("loading");
  const [mode, setMode] = useState<AuthMode>("api");
  const [user, setUser] = useState<AuthUser | null>(null);
  const [stores, setStores] = useState<Store[]>([]);
  const [store, setStore] = useState<Store | null>(null);
  const [permissions, setPermissions] = useState<string[]>([]);
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [dataKey, setDataKey] = useState(0);
  const clientRef = useRef<AdminClient | null>(null);
  const [client, setClient] = useState<AdminClient | null>(null);

  const applyStore = useCallback(async (next: Store) => {
    const storeClient = createAdmin(next.id);
    clientRef.current = storeClient;
    setClient(storeClient);
    setStore(next);
    writeStoreId(next.id);
    setDataKey((value) => value + 1);
    try {
      const me = await storeClient.auth.me();
      setPermissions(me.permissions);
    } catch {
      setPermissions([]);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;

    if (readDemoFlag()) {
      clientRef.current = null;
      setClient(null);
      setMode("demo");
      setUser(DEMO_USER);
      setStores([demoStore]);
      setStore(demoStore);
      setPermissions(["*"]);
      setStatus("authenticated");
      return () => {
        cancelled = true;
      };
    }

    const bootstrap = createAdmin("");
    clientRef.current = bootstrap;
    setClient(bootstrap);

    void (async () => {
      try {
        const me = await bootstrap.auth.me();
        if (cancelled) return;
        setUser(displayUser(me.user));
        const nextStores = await fetchStores(bootstrap);
        if (cancelled) return;
        setStores(nextStores);
        if (nextStores.length === 0) {
          setStatus("no-store");
          return;
        }
        const saved = readStoreId();
        const selected = nextStores.find((entry) => entry.id === saved) ?? nextStores[0] ?? null;
        if (selected === null) {
          setStatus("no-store");
          return;
        }
        await applyStore(selected);
        if (!cancelled) setStatus("authenticated");
      } catch (error) {
        if (cancelled) return;
        if (error instanceof ApiError && error.status === 401) {
          setConnectionError(null);
        } else {
          setConnectionError(errorMessage(error));
        }
        setStatus("anonymous");
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [applyStore]);

  const login = useCallback(
    async (email: string, password: string) => {
      const bootstrap = createAdmin("");
      setConnectionError(null);
      await bootstrap.auth.login({ email, password });
      const me = await bootstrap.auth.me();
      const nextStores = await fetchStores(bootstrap);
      clientRef.current = bootstrap;
      setClient(bootstrap);
      setMode("api");
      setUser(displayUser(me.user));
      setStores(nextStores);
      if (nextStores.length === 0) {
        setStatus("no-store");
        return;
      }
      const saved = readStoreId();
      const selected = nextStores.find((entry) => entry.id === saved) ?? nextStores[0] ?? null;
      if (selected === null) {
        setStatus("no-store");
        return;
      }
      await applyStore(selected);
      setStatus("authenticated");
    },
    [applyStore],
  );

  const logout = useCallback(async () => {
    writeDemoFlag(false);
    writeStoreId(null);
    if (mode === "api") {
      try {
        await createAdmin("").auth.logout();
      } catch {
        // The local session is cleared even when the server cannot be reached.
      }
    }
    clientRef.current = null;
    setClient(null);
    setMode("api");
    setUser(null);
    setStores([]);
    setStore(null);
    setPermissions([]);
    setStatus("anonymous");
  }, [mode]);

  const enterDemo = useCallback(() => {
    writeDemoFlag(true);
    clientRef.current = null;
    setClient(null);
    setMode("demo");
    setUser(DEMO_USER);
    setStores([demoStore]);
    setStore(demoStore);
    setPermissions(["*"]);
    setConnectionError(null);
    setStatus("authenticated");
    setDataKey((value) => value + 1);
  }, []);

  const selectStore = useCallback(
    (storeId: string) => {
      if (mode === "demo") return;
      const next = stores.find((entry) => entry.id === storeId);
      if (!next || next.id === store?.id) return;
      void applyStore(next);
    },
    [applyStore, mode, stores, store?.id],
  );

  const refreshStores = useCallback(async () => {
    const active = clientRef.current;
    if (!active) return;
    const nextStores = await fetchStores(active);
    setStores(nextStores);
    if (store) {
      const fresh = nextStores.find((entry) => entry.id === store.id);
      if (fresh) setStore(fresh);
      else {
        const fallback = nextStores[0] ?? null;
        if (fallback) await applyStore(fallback);
        else setStatus("no-store");
      }
    }
  }, [applyStore, store]);

  const can = useCallback(
    (permission: string) => {
      if (mode === "demo") return true;
      if (permissions.length === 0) return true;
      return permissions.includes("*") || permissions.includes(permission);
    },
    [mode, permissions],
  );

  const value = useMemo<AuthContextValue>(
    () => ({
      status,
      mode,
      user,
      stores,
      store,
      client,
      permissions,
      connectionError,
      dataKey,
      login,
      logout,
      enterDemo,
      selectStore,
      refreshStores,
      can,
    }),
    [
      status,
      mode,
      user,
      stores,
      store,
      client,
      permissions,
      connectionError,
      dataKey,
      login,
      logout,
      enterDemo,
      selectStore,
      refreshStores,
      can,
    ],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthContextValue {
  const context = useContext(AuthContext);
  if (!context) throw new Error("useAuth must be used within an AuthProvider");
  return context;
}
