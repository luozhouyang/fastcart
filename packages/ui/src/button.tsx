import type { ButtonHTMLAttributes } from "react";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary";
}

const VARIANTS = {
  primary: "bg-emerald-500 text-emerald-950 hover:bg-emerald-400",
  secondary: "border border-white/15 text-slate-100 hover:bg-white/5",
} as const;

export function Button({ variant = "primary", className = "", ...props }: ButtonProps) {
  return (
    <button
      type="button"
      className={`inline-flex items-center rounded-md px-3 py-2 text-sm font-medium transition ${VARIANTS[variant]} ${className}`}
      {...props}
    />
  );
}
