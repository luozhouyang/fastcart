import type { ThemeManifest } from "@fastcart/theme-sdk";
import theme from "../theme.json";

export const manifest = theme satisfies ThemeManifest;

export default manifest;
