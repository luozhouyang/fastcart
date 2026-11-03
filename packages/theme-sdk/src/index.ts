export interface ThemeManifest {
  id: string;
  name: string;
  version: string;
  runtime: string;
  entry: string;
  settings_schema?: string;
  assets_manifest?: string;
}

export interface ThemeSettingSchema {
  type: "object";
  properties: Record<string, unknown>;
  required?: string[];
}

export function defineTheme(manifest: ThemeManifest): ThemeManifest {
  return manifest;
}
