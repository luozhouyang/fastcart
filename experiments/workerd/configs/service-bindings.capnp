using Workerd = import "/workerd/workerd.capnp";

const config :Workerd.Config = (
  services = [
    (name = "app", worker = .appWorker),
    (name = "api", worker = .apiWorker),
  ],
  sockets = [
    (name = "http", address = "127.0.0.1:18790", http = (), service = "app"),
  ],
);

const appWorker :Workerd.Worker = (
  modules = [
    (name = "app.js", esModule = embed "../workers/service-app.js"),
  ],
  compatibilityDate = "2026-11-01",
  bindings = [
    (name = "API", service = "api"),
  ],
);

const apiWorker :Workerd.Worker = (
  modules = [
    (name = "api.js", esModule = embed "../workers/service-api.js"),
  ],
  compatibilityDate = "2026-11-01",
);
