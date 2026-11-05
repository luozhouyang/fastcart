import { hydrateRoot } from "react-dom/client";

import { PageRenderer } from "./components/page-renderer";
import { readHydrationState } from "./hydration";

const state = readHydrationState();
const container = document.getElementById("fastcart-root");

if (state && container) {
  hydrateRoot(
    container,
    <PageRenderer
      body={state.body}
      data={state.data}
      settings={state.settings}
      apiBaseUrl={state.apiBaseUrl}
      assetsBaseUrl={state.assetsBaseUrl}
      mode={state.mode}
    />,
  );
}
