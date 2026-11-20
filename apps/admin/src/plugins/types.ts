/** One admin extension entry returned by the host. */
export interface AdminExtension {
  target: string;
  title?: string | null;
  label?: string | null;
  icon?: string | null;
  route?: string | null;
  entry?: string | null;
  src?: string | null;
  height?: number | null;
  priority?: number | null;
  permission: string;
  plugin_id: string;
  plugin_name: string;
  plugin_version: string;
  digest: string;
  settings: Record<string, unknown>;
}

export interface AdminExtensionsResponse {
  extensions: AdminExtension[];
}
