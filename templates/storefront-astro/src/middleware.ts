import { defineMiddleware } from "astro:middleware";

export const onRequest = defineMiddleware((context, next) => {
  if (context.url.pathname === "/__fastcart/health") {
    return Response.json({ status: "ok", framework: "astro" });
  }
  return next();
});
