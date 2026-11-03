import handler, { createServerEntry } from "@tanstack/react-start/server-entry";

export default createServerEntry({
  fetch(request, options) {
    return handler.fetch(request, options);
  },
});
