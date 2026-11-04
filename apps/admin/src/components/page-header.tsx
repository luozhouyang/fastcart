import type { ReactNode } from "react";

export function PageHeader({
  title,
  description,
  actions,
}: {
  title: string;
  description: string;
  actions?: ReactNode;
}) {
  return (
    <header className="flex flex-col gap-5 pb-1 md:flex-row md:items-end md:justify-between">
      <div>
        <h1 className="font-display text-[40px] leading-[1.02] tracking-[-0.01em] text-foreground md:text-[46px]">
          {title}
        </h1>
        <p className="mt-2.5 max-w-xl text-[13px] leading-relaxed text-muted-foreground">
          {description}
        </p>
      </div>
      {actions ? <div className="flex flex-wrap items-center gap-2.5">{actions}</div> : null}
    </header>
  );
}
