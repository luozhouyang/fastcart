export type AdminCommand = "new-product" | "adjust-stock";

const EVENT = "fastcart:command";
let pending: AdminCommand | null = null;

/** Asks the current page (or the next one to mount) to open a dialog. */
export function requestCommand(command: AdminCommand): void {
  pending = command;
  if (typeof window !== "undefined") {
    window.dispatchEvent(new CustomEvent<AdminCommand>(EVENT, { detail: command }));
  }
}

export function consumePendingCommand(): AdminCommand | null {
  const command = pending;
  pending = null;
  return command;
}

export function onCommand(listener: (command: AdminCommand) => void): () => void {
  const handler = (event: Event) => {
    listener((event as CustomEvent<AdminCommand>).detail);
  };
  window.addEventListener(EVENT, handler);
  return () => window.removeEventListener(EVENT, handler);
}
