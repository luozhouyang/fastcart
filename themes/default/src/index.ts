import type { ThemeManifest } from "@fastcart/theme-sdk";
import theme from "../theme.json";

export { ContactForm } from "./components/contact-form";
export type { ContactFormProps } from "./components/contact-form";

export const manifest = theme satisfies ThemeManifest;

export default manifest;
