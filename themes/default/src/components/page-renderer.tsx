import type { PageBlock, PageBody, RenderMode, RenderRouteContext } from "@fastcart/theme-sdk";
import type { ReactNode } from "react";

import { CartProvider } from "../cart";
import { ThemeProvider } from "../context";
import { CartDrawer } from "./cart-drawer";
import { UnknownBlock } from "./unknown-block";
import { getThemeComponent } from "./registry";

/** Props accepted by the page renderer shared by SSR and hydration. */
export interface PageRendererProps {
  body: PageBody;
  /** Server-resolved data keyed by block id. */
  data?: Record<string, unknown> | null | undefined;
  /** Theme settings exposed to components through the theme context. */
  settings?: Record<string, unknown> | null | undefined;
  /** API origin used by interactive components such as the contact form. */
  apiBaseUrl?: string | null | undefined;
  /** Public base URL serving this bundle's client assets. */
  assetsBaseUrl?: string | null | undefined;
  mode?: RenderMode | string | null | undefined;
}

/**
 * Renders a page body: named zones (header before, footer after), then the
 * ordered content blocks. Data resolved by the host is merged into matching
 * block props by `props.id`.
 */
export function PageRenderer({
  body,
  data,
  settings,
  apiBaseUrl,
  assetsBaseUrl,
  mode,
}: PageRendererProps) {
  const zones: Record<string, PageBlock[]> = body?.zones ?? {};
  const entries = Object.entries(zones);
  const beforeZones = entries.filter(([name]) => name !== "footer");
  const afterZones = entries.filter(([name]) => name === "footer");

  const route = extractRoute(data);
  return (
    <ThemeProvider
      apiBaseUrl={apiBaseUrl}
      assetsBaseUrl={assetsBaseUrl}
      mode={mode}
      settings={settings}
      route={route}
    >
      <CartProvider apiBaseUrl={apiBaseUrl}>
        <div className="fc-page" data-theme="default">
          {beforeZones.map(([name, blocks]) => (
            <div key={name} className="fc-zone" data-zone={name}>
              {renderBlocks(blocks, data)}
            </div>
          ))}
          <main className="fc-content" data-zone="content">
            {renderBlocks(body?.content ?? [], data)}
          </main>
          {afterZones.map(([name, blocks]) => (
            <div key={name} className="fc-zone" data-zone={name}>
              {renderBlocks(blocks, data)}
            </div>
          ))}
        </div>
        <CartDrawer />
      </CartProvider>
    </ThemeProvider>
  );
}

function renderBlocks(
  blocks: PageBlock[],
  data: Record<string, unknown> | null | undefined,
): ReactNode {
  return blocks.map((block, index) => (
    <BlockRenderer key={blockKey(block, index)} block={block} data={data} index={index} />
  ));
}

function BlockRenderer({
  block,
  data,
  index,
}: {
  block: PageBlock;
  data: Record<string, unknown> | null | undefined;
  index: number;
}) {
  const props = mergeBlockData(block, data);
  const definition = getThemeComponent(block.type);
  if (!definition) {
    return <UnknownBlock type={block.type} />;
  }

  const Component = definition.component as unknown as (
    props: Record<string, unknown>,
  ) => ReactNode;
  const slotContent = renderSlots(block, data);
  const key = typeof props.id === "string" ? props.id : `${block.type}-${index}`;

  return (
    <div key={key} className="fc-block" data-block-type={block.type}>
      {slotContent.length > 0 ? (
        <Component {...props}>{slotContent}</Component>
      ) : (
        <Component {...props} />
      )}
    </div>
  );
}

function renderSlots(
  block: PageBlock,
  data: Record<string, unknown> | null | undefined,
): ReactNode[] {
  const slots = block.slots ?? {};
  return Object.entries(slots).flatMap(([name, children]) =>
    children.map((child, index) => (
      <div key={`${name}-${blockKey(child, index)}`} data-slot={name}>
        <BlockRenderer block={child} data={data} index={index} />
      </div>
    )),
  );
}

function blockKey(block: PageBlock, index: number): string {
  const id = block.props?.id;
  return typeof id === "string" && id.length > 0 ? id : `${block.type}-${index}`;
}

/**
 * Merges the data binding resolved for `props.id` into the block props.
 * Resolved data wins so a binding reference such as `{ form: { handle } }`
 * is replaced by the live object before components render.
 */
function mergeBlockData(
  block: PageBlock,
  data: Record<string, unknown> | null | undefined,
): Record<string, unknown> {
  const props = isRecord(block.props) ? block.props : {};
  const id = typeof props.id === "string" ? props.id : undefined;
  const resolved = id && isRecord(data) ? data[id] : undefined;
  return isRecord(resolved) ? { ...props, ...resolved } : { ...props };
}

/** Reads the host-resolved dynamic route from the reserved context entry. */
function extractRoute(data: Record<string, unknown> | null | undefined): RenderRouteContext | null {
  if (!isRecord(data) || !isRecord(data.context) || !isRecord(data.context.route)) {
    return null;
  }
  const route = data.context.route;
  return typeof route.kind === "string" && typeof route.handle === "string"
    ? (route as unknown as RenderRouteContext)
    : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
