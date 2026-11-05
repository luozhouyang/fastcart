import type { ThemeComponentDefinition } from "@fastcart/theme-sdk";
import { defineThemeComponent } from "@fastcart/theme-sdk";

import { ContactForm } from "./contact-form";
import { Heading } from "./heading";
import { Image } from "./image";
import { ProductGrid } from "./product-grid";
import { Text } from "./text";

/**
 * Registry of components page authors may reference by `block.type`.
 *
 * `defineThemeComponent` records the public name alongside the render
 * function. The editor manifest remains the source of truth for field
 * metadata, so the editor surface can load this module without duplicating
 * component definitions.
 */
function register(
  name: string,
  component: unknown,
): ThemeComponentDefinition<Record<string, unknown>> {
  if (typeof component !== "function") {
    throw new TypeError(`theme component ${name} must be a function`);
  }
  return defineThemeComponent(name, component as (props: Record<string, unknown>) => unknown);
}

export const components = {
  ContactForm: register("ContactForm", ContactForm),
  Text: register("Text", Text),
  Heading: register("Heading", Heading),
  Image: register("Image", Image),
  ProductGrid: register("ProductGrid", ProductGrid),
} as const;

/** Component names the default theme can render. */
export type RegisteredComponentName = keyof typeof components;

/** Looks up a registered component by block type. */
export function getThemeComponent(
  name: string,
): ThemeComponentDefinition<Record<string, unknown>> | undefined {
  return (components as Record<string, ThemeComponentDefinition<Record<string, unknown>>>)[name];
}
