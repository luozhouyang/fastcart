import { cloudflare } from "@cloudflare/vite-plugin";
import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import viteReact from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const assetPrefix = process.env.FASTCART_ASSET_PREFIX ?? "/_fastcart/assets/";
const base = assetPrefix.endsWith("/") ? assetPrefix : `${assetPrefix}/`;

export default defineConfig({
  base,
  plugins: [
    cloudflare({ viteEnvironment: { name: "ssr" } }),
    tanstackStart({ router: { basepath: "/" } }),
    viteReact(),
  ],
});
