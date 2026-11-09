import cloudflare from "@astrojs/cloudflare";
import { defineConfig } from "astro/config";

const assetPrefix = process.env.FASTCART_ASSET_PREFIX ?? "/_fastcart/assets/";
const base = assetPrefix.endsWith("/") ? assetPrefix : `${assetPrefix}/`;

export default defineConfig({
  output: "server",
  adapter: cloudflare(),
  build: {
    assetsPrefix: base,
    inlineStylesheets: "never",
  },
});
