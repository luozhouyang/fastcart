import { defineConfig } from "astro/config";
import tailwindcss from "@tailwindcss/vite";
import nimbus, { defineConfig as defineNimbusConfig } from "@cloudflare/nimbus-docs";
import { tableScroll } from "@cloudflare/nimbus-docs/markdown";

const nimbusConfig = defineNimbusConfig({
  // Canonical docs origin (no trailing slash). Drives canonical URLs,
  // absolute OG image URLs, robots.txt, sitemap, and the links in /llms.txt.
  // Change this if the site is served from another host.
  site: "https://docs.fastcart.dev",
  // Used for <title>, the home H1, and OG metadata.
  title: "Fastcart Docs",
  // One-line description used for meta + OG metadata.
  description:
    "Documentation for Fastcart, a self-hosted commerce platform built with Rust and React.",
  locale: "en",
  // Shown as the GitHub link in the header.
  github: "https://github.com/luozhouyang/fastcart",
  socialImageAlt: "Fastcart documentation preview",
  // The home page leads the sidebar; every other page joins it automatically.
  sidebar: {
    items: ["index", { autogenerate: { collection: "docs" } }],
  },
});

export default defineConfig({
  // nimbus:adapter
  output: "static",
  // The SSR guide moved under How it works; keep the old URL working.
  redirects: {
    "/storefront-ssr": "/how-it-works/storefront-ssr",
  },
  // Tailwind v4 via its Vite plugin (the integration Astro recommends for
  // Tailwind v4 — replaces the PostCSS plugin, which doesn't build under
  // Astro 7's Vite 8 bundler).
  vite: {
    plugins: [tailwindcss()],
  },
  // Hover-prefetch link targets so full-page navigations feel instant without
  // a client-side router.
  prefetch: {
    prefetchAll: true,
    defaultStrategy: "hover",
  },
  integrations: [
    nimbus(nimbusConfig, {
      // Authoring rules are opt-in by design — your repo, your taste. The
      // two below are the load-bearing pair: frontmatter has to validate
      // against the content schema for the page to render properly, and
      // broken internal links are 404s for your readers. Add the others
      // (heading hierarchy, code-block language, style, etc.) when you're
      // ready to enforce them — see `nimbus-docs lint --help`.
      rules: {
        "nimbus/frontmatter-shape": "error",
        "nimbus/internal-link": "error",
      },
      markdown: {
        // Wrap wide tables so they scroll instead of overflowing the page
        // (styled by `.nb-table-scroll` in src/styles/prose.css).
        hastPlugins: [tableScroll()],
        // `## Install {#install}` gives a heading a fixed id. A heading that
        // ends in other `{…}` text, such as `{account_id}` in a .md file, is
        // read as attributes too; remove this to keep it as text.
        features: { headingAttributes: true },
      },
    }),
  ],
});
