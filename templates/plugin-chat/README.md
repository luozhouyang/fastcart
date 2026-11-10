# Live Chat example plugin

A minimal P3/P4 example:

- `plugin.json` declares one nav entry, two admin slots, and one storefront embed.
- `worker/index.js` answers the admin RPC method `conversations.list`.
- `ui/settings.html` is the sandboxed iframe page; it speaks the `postMessage` protocol.
- `storefront/boot.js` mounts a Shadow DOM chat bubble and calls `/apps/chat-widget/api/hello`.

Package the directory as `tar.gz` with `plugin.json` at the archive root, then install it from **Plugins** in the admin.
