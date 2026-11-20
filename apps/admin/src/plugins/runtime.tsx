import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import { useAuth } from "@/lib/auth";
import { fetchPluginExtensions } from "@/lib/plugins";
import type { AdminExtension } from "@/plugins/types";

interface PluginExtensionsValue {
  extensions: AdminExtension[];
  loading: boolean;
  error: string | null;
  refresh: () => void;
}

const PluginExtensionsContext = createContext<PluginExtensionsValue | null>(null);

export function PluginExtensionsProvider({ children }: { children: ReactNode }) {
  const { store, mode } = useAuth();
  const [extensions, setExtensions] = useState<AdminExtension[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);

  const refresh = useCallback(() => setRevision((value) => value + 1), []);

  useEffect(() => {
    if (!store || mode !== "api") {
      setExtensions([]);
      setLoading(false);
      return;
    }
    const controller = new AbortController();
    setLoading(true);
    fetchPluginExtensions(store.id, controller.signal)
      .then((next) => {
        setExtensions(next);
        setError(null);
      })
      .catch((cause: unknown) => {
        if (controller.signal.aborted) return;
        setExtensions([]);
        setError(cause instanceof Error ? cause.message : "Could not load plugin extensions");
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [store, mode, revision]);

  const value = useMemo(
    () => ({ extensions, loading, error, refresh }),
    [extensions, loading, error, refresh],
  );
  return (
    <PluginExtensionsContext.Provider value={value}>{children}</PluginExtensionsContext.Provider>
  );
}

export function usePluginExtensions(): PluginExtensionsValue {
  const value = useContext(PluginExtensionsContext);
  if (!value) {
    throw new Error("usePluginExtensions must be used inside PluginExtensionsProvider");
  }
  return value;
}
