import { useState, type FormEvent } from "react";
import { ArrowRight, ChevronsRight, CircleAlert, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ThemeToggle } from "@/components/theme-toggle";
import { useAuth } from "@/lib/auth";
import { errorMessage } from "@/lib/api";

function BrandMark({ inverted = false }: { inverted?: boolean }) {
  return (
    <span className="inline-flex items-center gap-2.5">
      <span
        className={
          inverted
            ? "flex size-8 items-center justify-center rounded-[9px] bg-[#f7f8f3] text-[#151815]"
            : "flex size-8 items-center justify-center rounded-[9px] bg-primary text-primary-foreground"
        }
      >
        <ChevronsRight className="size-4" strokeWidth={2.4} aria-hidden="true" />
      </span>
      <span
        className={
          inverted
            ? "text-[17px] font-medium tracking-[-0.03em] text-[#f7f8f3]"
            : "text-[17px] font-medium tracking-[-0.03em] text-foreground"
        }
      >
        fastcart
      </span>
    </span>
  );
}

export function SplashScreen() {
  return (
    <div className="flex min-h-screen flex-col bg-background">
      <div className="px-6 py-6 sm:px-10">
        <BrandMark />
      </div>
      <div className="flex flex-1 flex-col items-center justify-center gap-4 pb-24">
        <Loader2 className="size-5 animate-spin text-brand" aria-hidden="true" />
        <p className="font-mono text-[11px] tracking-wide text-faint">Loading store</p>
      </div>
    </div>
  );
}

export function LoginPage() {
  const { login, enterDemo, connectionError } = useAuth();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await login(email.trim(), password);
    } catch (cause) {
      setError(errorMessage(cause));
      setBusy(false);
    }
  };

  return (
    <div className="grid min-h-screen bg-background lg:grid-cols-[minmax(0,0.88fr)_minmax(0,1.12fr)]">
      <aside className="relative hidden overflow-hidden bg-[#151815] p-10 text-[#f7f8f3] lg:flex lg:flex-col">
        <BrandMark inverted />

        <div className="mt-auto max-w-[420px] pb-16">
          <p className="font-mono text-[10.5px] tracking-[0.14em] text-[#8e968d]">FASTCART ADMIN</p>
          <h1 className="mt-5 font-display text-[52px] leading-[0.98] tracking-[-0.015em] text-[#f4f5ef]">
            Every order,
            <br />
            accounted for.
          </h1>
          <p className="mt-6 max-w-[340px] text-[13px] leading-relaxed text-[#a9b0a6]">
            A dispatch desk for self-hosted commerce. Orders, stock and customers in one quiet
            place.
          </p>

          <dl className="mt-12 border-t border-white/10">
            {[
              ["Orders", "Process and fulfil"],
              ["Inventory", "Track every location"],
              ["Customers", "Know who returns"],
            ].map(([term, detail]) => (
              <div
                key={term}
                className="flex items-baseline justify-between border-b border-white/10 py-3.5"
              >
                <dt className="text-[12.5px] font-medium text-[#e4e6df]">{term}</dt>
                <dd className="font-mono text-[10.5px] text-[#7f877d]">{detail}</dd>
              </div>
            ))}
          </dl>
        </div>

        <p className="font-mono text-[10.5px] text-[#6f776d]">Self-hosted · v0.1.0</p>
      </aside>

      <main className="relative flex flex-col px-6 py-6 sm:px-10">
        <div className="flex items-center justify-between lg:justify-end">
          <span className="lg:hidden">
            <BrandMark />
          </span>
          <ThemeToggle />
        </div>

        <div className="flex flex-1 items-center justify-center py-12">
          <div className="w-full max-w-[356px] animate-enter">
            <h1 className="font-display text-[42px] leading-[1.02] tracking-[-0.01em] text-foreground">
              Sign in
            </h1>
            <p className="mt-3 text-[13px] leading-relaxed text-muted-foreground">
              Use your Fastcart administrator account to open the dispatch desk.
            </p>

            {connectionError ? (
              <div className="mt-6 flex items-start gap-2.5 rounded-xl border border-warning/25 bg-warning-soft px-3.5 py-3 text-[12px] leading-relaxed text-warning">
                <CircleAlert className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                <span>
                  {connectionError} You can still review the interface with the demo store.
                </span>
              </div>
            ) : null}

            <form className="mt-8 space-y-5" onSubmit={(event) => void submit(event)}>
              <div className="space-y-2">
                <Label htmlFor="email">Email</Label>
                <Input
                  id="email"
                  type="email"
                  autoComplete="username"
                  placeholder="you@store.com"
                  value={email}
                  onChange={(event) => setEmail(event.target.value)}
                  required
                  disabled={busy}
                />
              </div>

              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <Label htmlFor="password">Password</Label>
                </div>
                <Input
                  id="password"
                  type="password"
                  autoComplete="current-password"
                  placeholder="••••••••"
                  value={password}
                  onChange={(event) => setPassword(event.target.value)}
                  required
                  disabled={busy}
                />
              </div>

              {error ? (
                <div
                  role="alert"
                  className="flex items-start gap-2.5 rounded-xl border border-destructive/20 bg-destructive/8 px-3.5 py-3 text-[12px] leading-relaxed text-destructive"
                >
                  <CircleAlert className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
                  <span>{error}</span>
                </div>
              ) : null}

              <Button type="submit" size="lg" className="w-full" disabled={busy}>
                {busy ? (
                  <>
                    <Loader2 className="size-4 animate-spin" aria-hidden="true" />
                    Signing in
                  </>
                ) : (
                  "Sign in"
                )}
              </Button>
            </form>

            <div className="my-6 flex items-center gap-3">
              <span className="h-px flex-1 bg-border" />
              <span className="text-[10.5px] text-faint">or</span>
              <span className="h-px flex-1 bg-border" />
            </div>

            <Button variant="outline" size="lg" className="w-full" onClick={enterDemo}>
              Explore the demo store
              <ArrowRight className="size-4" aria-hidden="true" />
            </Button>

            <p className="mt-8 text-center text-[11px] leading-relaxed text-faint">
              Sessions are stored in an HTTP-only cookie. CSRF is checked on every change.
            </p>
          </div>
        </div>
      </main>
    </div>
  );
}

export function NoStorePage() {
  const { user, logout, enterDemo } = useAuth();

  return (
    <div className="flex min-h-screen flex-col bg-background">
      <div className="flex items-center justify-between px-6 py-6 sm:px-10">
        <BrandMark />
        <ThemeToggle />
      </div>
      <main className="flex flex-1 items-center justify-center px-6 pb-24">
        <div className="w-full max-w-[460px] animate-enter">
          <p className="font-mono text-[10.5px] tracking-[0.14em] text-faint">NO STORE YET</p>
          <h1 className="mt-4 font-display text-[44px] leading-[1.02] text-foreground">
            Create your first store
          </h1>
          <p className="mt-3 text-[13px] leading-relaxed text-muted-foreground">
            {user?.email ?? "This account"} is signed in, but it is not attached to a store.
            Provision one from the Fastcart CLI, then reload this page.
          </p>
          <pre className="mt-6 overflow-x-auto rounded-xl border border-border bg-card px-4 py-3.5 font-mono text-[12px] text-ink-soft shadow-soft">
            <code>{`fastcart store create --name "My store" --slug my-store --domain store.example.com`}</code>
          </pre>
          <div className="mt-6 flex flex-wrap gap-2.5">
            <Button variant="outline" onClick={enterDemo}>
              Explore the demo
            </Button>
            <Button variant="ghost" onClick={() => void logout()}>
              Sign out
            </Button>
          </div>
        </div>
      </main>
    </div>
  );
}
