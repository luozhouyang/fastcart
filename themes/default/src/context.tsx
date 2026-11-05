import { createContext, useContext, type ReactNode } from "react";

import type { RenderMode, RenderRouteContext } from "@fastcart/theme-sdk";

/**
 * Runtime values shared by server rendering and client hydration.
 *
 * `apiBaseUrl` is the origin (optionally with a path prefix) where the public
 * storefront API is reachable. It is empty in the common case where the theme
 * is served from the same origin as `/api/v1/store/*`; a reverse proxy or a
 * cross-origin deployment can override it through theme settings.
 */
export interface ThemeRuntimeContextValue {
  apiBaseUrl?: string;
  assetsBaseUrl?: string;
  mode?: RenderMode;
  settings: Record<string, unknown>;
  /** Resource resolved for a dynamic route such as a product page. */
  route?: RenderRouteContext;
}

const defaultContext: ThemeRuntimeContextValue = { settings: {} };

export const ThemeContext = createContext<ThemeRuntimeContextValue>(defaultContext);

export interface ThemeProviderProps {
  apiBaseUrl?: string | null | undefined;
  assetsBaseUrl?: string | null | undefined;
  mode?: RenderMode | string | null | undefined;
  settings?: Record<string, unknown> | null | undefined;
  route?: RenderRouteContext | null | undefined;
  children?: ReactNode;
}

/** Provides runtime values to every component in the page tree. */
export function ThemeProvider({
  apiBaseUrl,
  assetsBaseUrl,
  mode,
  settings,
  route,
  children,
}: ThemeProviderProps) {
  const value: ThemeRuntimeContextValue = {
    settings: settings ?? {},
    ...(apiBaseUrl ? { apiBaseUrl } : {}),
    ...(assetsBaseUrl ? { assetsBaseUrl } : {}),
    ...(mode === "production" || mode === "preview" || mode === "editor" ? { mode } : {}),
    ...(route ? { route } : {}),
  };
  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

/** Returns the current theme runtime context. */
export function useThemeRuntime(): ThemeRuntimeContextValue {
  return useContext(ThemeContext);
}
