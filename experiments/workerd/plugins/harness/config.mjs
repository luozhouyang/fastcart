export function buildConfig({ port, commercePort, loaderId = "plugins" }) {
  return `using Workerd = import "/workerd/workerd.capnp";

const config :Workerd.Config = (
  services = [
    (name = "gateway", worker = .gateway),
    (name = "bridge", worker = .bridge),
    (name = "commerce", external = (address = "127.0.0.1:${commercePort}", http = ())),
  ],
  sockets = [
    (name = "http", address = "127.0.0.1:${port}", http = (), service = "gateway"),
  ],
);

const gateway :Workerd.Worker = (
  modules = [(name = "gateway.js", esModule = embed "gateway.js")],
  compatibilityDate = "2026-11-01",
  bindings = [
    (name = "LOADER", workerLoader = (id = "${loaderId}")),
    (name = "BRIDGE", service = "bridge"),
    (name = "COMMERCE", service = "commerce"),
  ],
);

const bridge :Workerd.Worker = (
  modules = [(name = "bridge.js", esModule = embed "bridge.js")],
  compatibilityDate = "2026-11-01",
  bindings = [(name = "UPSTREAM", service = "commerce")],
);
`;
}
