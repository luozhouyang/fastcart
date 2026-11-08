import type { ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { ShieldAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useAuth } from "@/lib/auth";
import type { Permission } from "@/lib/permissions";

export interface PermissionGateProps {
  permission: Permission;
  children: ReactNode;
}

/**
 * Renders a page only when the signed-in account has its read permission.
 *
 * The API enforces every permission; this gate prevents a page from making
 * requests that can only fail and gives the operator a clear explanation.
 * Demo mode reports every permission as granted.
 */
export function PermissionGate({ permission, children }: PermissionGateProps) {
  const { can, mode } = useAuth();
  if (mode === "demo" || can(permission)) {
    return <>{children}</>;
  }
  return (
    <div className="mx-auto max-w-xl rounded-lg border border-border bg-card p-8 text-center">
      <ShieldAlert className="mx-auto size-9 text-muted-foreground" aria-hidden="true" />
      <h1 className="mt-4 font-display text-[28px] leading-tight text-foreground">
        You don&apos;t have access
      </h1>
      <p className="mt-2 text-[13px] text-muted-foreground">
        This page requires the <span className="font-mono">{permission}</span> permission. Ask a
        store owner or administrator to grant it.
      </p>
      <Button asChild className="mt-6" variant="outline">
        <Link to="/">Back to the overview</Link>
      </Button>
    </div>
  );
}
