/**
 * Public contracts for Fastcart storefront themes.
 *
 * The types here mirror the Rust contracts in `crates/storefront` and
 * `crates/storefront-runtime/runtime-api`. Keep optional fields optional: a
 * theme bundle must keep working when the host sends an older payload.
 */

/** Bundle-relative references to a theme's editor surface. */
export interface ThemeEditorRef {
  /** ESM entry that exports the theme component registry. */
  entry: string;
  /** JSON file describing categories, components and fields. */
  manifest: string;
  /** Editor manifest schema version understood by the theme. */
  schema_version?: number;
}

/** One platform capability a theme depends on. */
export interface ThemeCapabilityRequirement {
  /** Capability identifier. */
  id: string;
  /** Required capability version. */
  version: number;
  /** Whether activation may proceed when the capability is missing. */
  optional?: boolean;
}

/** Platform contracts and capabilities a theme depends on. */
export interface ThemeRequirements {
  /** Required storefront data API version. */
  data_api?: number;
  /** Optional or required plugin capabilities. */
  capabilities?: ThemeCapabilityRequirement[];
}

/** Describes a trusted storefront theme bundle. */
export interface ThemeManifest {
  id: string;
  name: string;
  version: string;
  runtime: string;
  entry: string;
  settings_schema?: string;
  assets_manifest?: string;
  editor?: ThemeEditorRef;
  requires?: ThemeRequirements;
  /** Dynamic routes keyed by template page type. */
  routes?: Record<string, string>;
}

export interface ThemeSettingSchema {
  type: "object";
  properties: Record<string, unknown>;
  required?: string[];
}

/** Marks a manifest as a theme without changing its runtime value. */
export function defineTheme(manifest: ThemeManifest): ThemeManifest {
  return manifest;
}

// ---------------------------------------------------------------------------
// Editor manifest contract (mirrors crates/storefront/src/editor.rs)
// ---------------------------------------------------------------------------

/** Field types accepted by the editor contract. */
export type EditorFieldType =
  | "text"
  | "textarea"
  | "number"
  | "boolean"
  | "select"
  | "object"
  | "array"
  | "data_source"
  | "asset"
  | "custom";

/** Data sources fixed by the platform vocabulary. */
export type EditorDataSource =
  | "collection"
  | "product"
  | "product_list"
  | "page"
  | "navigation"
  | "asset"
  | "search"
  | "inquiry_form";

/** One selectable option on a `select` field. */
export interface EditorFieldOption {
  value: string;
  label: string;
}

/** One editable field on a component or the page root. */
export interface EditorField {
  type: EditorFieldType | (string & {});
  /** Human-readable label shown by the editor. */
  label?: string;
  /** Optional help text shown under the input. */
  description?: string;
  /** Optional picker configuration for `data_source` and `asset` fields. */
  picker?: unknown;
  /** Data source kind for `data_source` and `asset` fields. */
  source?: EditorDataSource | (string & {});
  required?: boolean;
  min?: number;
  max?: number;
  options?: EditorFieldOption[];
  default?: unknown;
  fields?: Record<string, EditorField>;
  items?: EditorField;
}

/** One nested region declared by a component. */
export interface EditorSlot {
  name: string;
  title?: string;
  /** Component types allowed in the slot; omitted allows every component. */
  allowed_components?: string[];
}

/** One page component exposed by a theme. */
export interface ThemeEditorComponent {
  title: string;
  /** Optional short description shown in the palette or inspector. */
  description?: string;
  /** Optional icon name understood by the admin editor. */
  icon?: string;
  category?: string;
  fields?: Record<string, EditorField>;
  defaultProps?: unknown;
  slots?: EditorSlot[];
}

/** One named page zone (header, footer, sidebar) exposed by a theme. */
export interface ThemeEditorZone {
  title: string;
  /** Component types allowed in the zone; omitted allows every component. */
  allowed_components?: string[];
}

/** One editor palette category. */
export interface ThemeEditorCategory {
  title: string;
  components?: string[];
}

/** Root-level page fields. */
export interface ThemeEditorRoot {
  fields?: Record<string, EditorField>;
}

/** The structural contract shared by the editor, validation and AI tools. */
export interface ThemeEditorManifest {
  schema_version?: number;
  categories?: Record<string, ThemeEditorCategory>;
  components?: Record<string, ThemeEditorComponent>;
  root?: ThemeEditorRoot;
  /** Named page zones (header, footer, sidebar) available to authors. */
  zones?: Record<string, ThemeEditorZone>;
}

// ---------------------------------------------------------------------------
// Page body contract (mirrors crates/storefront/src/page.rs)
// ---------------------------------------------------------------------------

/** Theme metadata captured with a page body. */
export interface PageThemeRef {
  id: string;
  version: string;
  schema_version?: number;
}

/** One block in a page body. */
export interface PageBlock {
  /** Registered component type, for example `ContactForm`. */
  type: string;
  /** Component props. `props.id` is the stable block identifier. */
  props?: Record<string, unknown>;
  /** Nested slot content keyed by declared slot name. */
  slots?: Record<string, PageBlock[]>;
}

/** Root property values. */
export interface PageRoot {
  props?: unknown;
}

/** Versioned page definition consumed by the renderer. */
export interface PageBody {
  schema_version?: number;
  theme?: PageThemeRef;
  /** Ordered page blocks. */
  content?: PageBlock[];
  root?: PageRoot;
  /** Named top-level regions such as header or footer. */
  zones?: Record<string, PageBlock[]>;
}

/**
 * A typed reference stored inside page props.
 *
 * Bindings are resolved on the server for every render; the resolved value is
 * delivered in `RenderPayload.data` keyed by block id.
 */
export type DataBinding =
  | { source: "collection"; id: string; limit?: number }
  | { source: "product"; id: string }
  | {
      source: "product_list";
      collection_id?: string;
      tag?: string;
      sort?: "title_asc" | "title_desc" | "created_at_asc" | "created_at_desc";
      limit?: number;
    }
  | { source: "page"; id: string }
  | { source: "navigation"; id: string }
  | { source: "asset"; id: string }
  | { source: "search"; query: string; limit?: number }
  | { source: "inquiry_form"; handle: string; id?: string; label?: string };

// ---------------------------------------------------------------------------
// Render payload contract (mirrors crates/storefront-runtime/runtime-api)
// ---------------------------------------------------------------------------

/** Trusted store context resolved by the host. */
export interface RenderStoreContext {
  id: string;
  domain: string;
  locale: string;
  currency: string;
  timezone: string;
}

/** The active theme bundle supplied to the runtime. */
export interface RenderThemeContext {
  id: string;
  version: string;
  runtime: string;
  bundle_path: string;
  entry: string;
  settings: unknown;
  assets_manifest?: unknown;
  assets_base_url?: string | null;
}

/** The content page selected for this request. */
export interface RenderPageContext {
  id: string;
  handle: string;
  status: string;
  page_type?: string;
  title?: string;
  seo?: unknown;
  body: PageBody | unknown;
}

/**
 * Resource resolved for a dynamic theme route.
 *
 * The host injects this into the reserved `context` data entry, so page
 * components read it from the theme runtime rather than from block props.
 */
export interface RenderRouteContext {
  /** Route kind declared by the theme manifest, e.g. `product`. */
  kind: string;
  /** Handle captured from the request path. */
  handle: string;
  /**
   * Resolved resource. A product route carries `product`, `subtitle`,
   * `description` and `images`; a collection route carries `collection` and
   * `products`.
   */
  resource: Record<string, unknown>;
}

/** One plugin capability granted to this request. */
export interface RenderCapabilityGrant {
  version: number;
  endpoint: string;
  token: string;
  config: unknown;
}

/** How the request treats draft content and capabilities. */
export type RenderMode = "production" | "preview" | "editor";

/** Request body sent by the Rust `NodeRuntime` to the sidecar `/render`. */
export interface RenderPayload {
  request_id: string;
  store: RenderStoreContext;
  method: string;
  url: string;
  headers: [string, string][];
  /** Base64 encoded request body, or null. */
  body: string | null;
  theme: RenderThemeContext | null;
  page: RenderPageContext | null;
  data: Record<string, unknown> | null;
  capabilities: Record<string, RenderCapabilityGrant>;
  mode: RenderMode;
  timeout_ms: number;
  /**
   * Whether the host can consume a streamed body.
   *
   * When false, a theme that returns a stream is buffered by the sidecar and
   * delivered through the buffered JSON contract. Older hosts omit the field.
   */
  streaming?: boolean;
  /** Total time budget for a streamed body, in milliseconds. */
  stream_timeout_ms?: number;
}

/**
 * A body a theme may return.
 *
 * Strings and bytes are buffered. Async iterables and web `ReadableStream`s are
 * streamed chunk by chunk when the host accepts it, and buffered otherwise.
 */
export type RenderBody =
  string | Uint8Array | ReadableStream<Uint8Array> | AsyncIterable<Uint8Array> | null;

/** Response returned by a theme's `render` export. */
export interface RenderResult {
  status: number;
  headers?: [string, string][];
  /** Raw response body, optionally streamed. */
  body: RenderBody;
}

// ---------------------------------------------------------------------------
// Component helpers
// ---------------------------------------------------------------------------

/** The minimal shape of a renderable theme component (React or otherwise). */
export type ThemeComponent<P = Record<string, unknown>> = (props: P) => unknown;

/** A theme component plus the editor metadata that describes it. */
export interface ThemeComponentDefinition<P = Record<string, unknown>> {
  name: string;
  component: ThemeComponent<P>;
  editor?: ThemeEditorComponent;
}

/**
 * Registers a component with the theme registry and (optionally) the editor
 * manifest. The helper is intentionally runtime-free so it can be used in
 * both SSR and client bundles.
 */
export function defineThemeComponent<P>(
  name: string,
  component: ThemeComponent<P>,
  editor?: ThemeEditorComponent,
): ThemeComponentDefinition<P>;
export function defineThemeComponent<P>(
  definition: ThemeComponentDefinition<P>,
): ThemeComponentDefinition<P>;
export function defineThemeComponent<P>(
  nameOrDefinition: string | ThemeComponentDefinition<P>,
  component?: ThemeComponent<P>,
  editor?: ThemeEditorComponent,
): ThemeComponentDefinition<P> {
  if (typeof nameOrDefinition !== "string") {
    return nameOrDefinition;
  }
  if (component === undefined) {
    throw new Error("defineThemeComponent requires a component function");
  }
  return editor === undefined
    ? { name: nameOrDefinition, component }
    : { name: nameOrDefinition, component, editor };
}
