import type { ThemeManifest } from "@fastcart/theme-sdk";
import theme from "../theme.json";

export { ContactForm } from "./components/contact-form";
export type { ContactFormProps } from "./components/contact-form";
export { CollectionDetail } from "./components/collection-detail";
export type { CollectionDetailProps } from "./components/collection-detail";
export { Heading } from "./components/heading";
export type { HeadingProps } from "./components/heading";
export { Image } from "./components/image";
export type { ImageAsset, ImageProps } from "./components/image";
export { PageRenderer } from "./components/page-renderer";
export type { PageRendererProps } from "./components/page-renderer";
export { ProductDetail } from "./components/product-detail";
export type { ProductDetailProps } from "./components/product-detail";
export { ProductGrid } from "./components/product-grid";
export type {
  ProductGridItem,
  ProductGridPrice,
  ProductGridProps,
} from "./components/product-grid";
export { getThemeComponent, components as componentRegistry } from "./components/registry";
export { Text } from "./components/text";
export type { TextProps } from "./components/text";
export { UnknownBlock } from "./components/unknown-block";
export type { UnknownBlockProps } from "./components/unknown-block";
export { ThemeProvider, useThemeRuntime } from "./context";
export type { ThemeProviderProps, ThemeRuntimeContextValue } from "./context";

export const manifest = theme satisfies ThemeManifest;

export default manifest;
