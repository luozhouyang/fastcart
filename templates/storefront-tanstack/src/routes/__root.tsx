import { HeadContent, Scripts, createRootRoute } from "@tanstack/react-router";
import type { ReactNode } from "react";

export const Route = createRootRoute({
  head: () => ({
    meta: [
      { charSet: "utf-8" },
      { name: "viewport", content: "width=device-width, initial-scale=1" },
      { title: "FastCart TanStack Storefront" },
    ],
  }),
  shellComponent: RootDocument,
});

function RootDocument({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <head>
        <HeadContent />
        <script type="application/fastcart-extensions" data-slot="head" />
      </head>
      <body>
        {children}
        <Scripts />
        <script type="application/fastcart-extensions" data-slot="body" />
      </body>
    </html>
  );
}
