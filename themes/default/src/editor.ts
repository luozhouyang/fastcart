import { ContactForm } from "./components/contact-form";
import { Heading } from "./components/heading";
import { Image } from "./components/image";
import { ProductGrid } from "./components/product-grid";
import { components as registry } from "./components/registry";
import { Text } from "./components/text";

/**
 * Component registry consumed by the theme editor surface.
 *
 * The admin editor reads structural metadata from `editor.manifest.json`; this
 * entry is the render-side counterpart for hosts that load theme components.
 * `registry` additionally carries the public component names recorded by
 * `defineThemeComponent`.
 */
export const components = {
  ContactForm,
  Text,
  Heading,
  Image,
  ProductGrid,
};

export const definitions = registry;

export default components;
