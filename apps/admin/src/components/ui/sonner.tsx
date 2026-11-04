import { Toaster as Sonner } from "sonner";

type Theme = "light" | "dark";

export function Toaster({ theme }: { theme: Theme }) {
  return (
    <Sonner
      theme={theme}
      position="bottom-right"
      gap={8}
      visibleToasts={3}
      closeButton
      toastOptions={{
        classNames: {
          toast:
            "group !rounded-xl !border !border-border !bg-card !text-card-foreground !shadow-lift !font-sans",
          description: "!text-muted-foreground",
          actionButton: "!bg-primary !text-primary-foreground",
          cancelButton: "!bg-secondary !text-secondary-foreground",
          closeButton: "!border-border !bg-card !text-muted-foreground hover:!text-foreground",
          title: "!text-[13px] !font-medium",
        },
      }}
    />
  );
}
