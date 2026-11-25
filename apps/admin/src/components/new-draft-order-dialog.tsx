import { useState } from "react";
import { Loader2 } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";

/**
 * Authors a one-line draft order.
 *
 * More lines are added through an order edit after creation, which is the same
 * path operators use to change an existing draft.
 */
export function NewDraftOrderDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: () => void;
}) {
  const { client, store } = useAuth();
  const [email, setEmail] = useState("");
  const [country, setCountry] = useState("");
  const [title, setTitle] = useState("");
  const [price, setPrice] = useState("");
  const [quantity, setQuantity] = useState("1");
  const [shipping, setShipping] = useState("0");
  const [busy, setBusy] = useState(false);

  const reset = () => {
    setEmail("");
    setCountry("");
    setTitle("");
    setPrice("");
    setQuantity("1");
    setShipping("0");
  };

  const submit = async () => {
    if (!client || busy) return;
    const unitPrice = Math.round(Number(price) * 100);
    if (!Number.isFinite(unitPrice) || unitPrice < 0 || price.trim().length === 0) {
      toast.error("Enter a unit price of zero or more.");
      return;
    }
    const parsedQuantity = Number(quantity);
    if (!Number.isInteger(parsedQuantity) || parsedQuantity < 1) {
      toast.error("Enter a whole quantity greater than zero.");
      return;
    }
    const shippingTotal = Math.round(Number(shipping || "0") * 100);
    if (!Number.isFinite(shippingTotal) || shippingTotal < 0) {
      toast.error("Enter a shipping amount of zero or more.");
      return;
    }
    if (title.trim().length === 0) {
      toast.error("Enter the item title.");
      return;
    }
    setBusy(true);
    try {
      await client.orders.createDraft({
        email: email.trim(),
        currency_code: store?.currency_code ?? "USD",
        items: [
          {
            product_title: title.trim(),
            unit_price: unitPrice,
            quantity: parsedQuantity,
            requires_shipping: true,
          },
        ],
        shipping_address:
          country.trim().length > 0 ? { country_code: country.trim().toUpperCase() } : {},
        billing_address: {},
        shipping_total: shippingTotal,
      });
      toast.success("Draft order created.");
      reset();
      onCreated();
      onOpenChange(false);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !busy && onOpenChange(next)}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>New draft order</DialogTitle>
          <DialogDescription>
            Start with one line and an address; add or change lines with an order edit.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-2">
            <Label htmlFor="draft-email">Customer email</Label>
            <Input
              id="draft-email"
              type="email"
              placeholder="buyer@example.com"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
              disabled={busy}
            />
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="draft-title">Item</Label>
              <Input
                id="draft-title"
                placeholder="Linen throw"
                value={title}
                onChange={(event) => setTitle(event.target.value)}
                disabled={busy}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="draft-country">Country</Label>
              <Input
                id="draft-country"
                placeholder="DE"
                maxLength={2}
                value={country}
                onChange={(event) => setCountry(event.target.value)}
                disabled={busy}
              />
            </div>
          </div>
          <div className="grid grid-cols-3 gap-3">
            <div className="space-y-2">
              <Label htmlFor="draft-price">Unit price</Label>
              <Input
                id="draft-price"
                type="number"
                min="0"
                step="0.01"
                value={price}
                onChange={(event) => setPrice(event.target.value)}
                disabled={busy}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="draft-quantity">Quantity</Label>
              <Input
                id="draft-quantity"
                type="number"
                min="1"
                step="1"
                value={quantity}
                onChange={(event) => setQuantity(event.target.value)}
                disabled={busy}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="draft-shipping">Shipping</Label>
              <Input
                id="draft-shipping"
                type="number"
                min="0"
                step="0.01"
                value={shipping}
                onChange={(event) => setShipping(event.target.value)}
                disabled={busy}
              />
            </div>
          </div>
          <p className="text-[11px] text-muted-foreground">
            A country code enables tax pricing; without one the draft is created untaxed.
          </p>
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
            Cancel
          </Button>
          <Button onClick={() => void submit()} disabled={busy}>
            {busy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
            Create draft
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
