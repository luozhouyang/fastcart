---
type: ADR
title: Order edits settle their stock, money and fulfillment state
description: Confirming an order edit moves inventory in the same transaction, records the price difference it settles, and re-derives fulfillment state; collecting or refunding the difference stays an explicit idempotent call.
owner: luozhouyang
status: Accepted
implementation: complete
timestamp: 2026-11-24T00:00:00Z
---

# 0011. Order edits settle their stock, money and fulfillment state

## Context

[ADR 0009](0009-order-edits-materialize-line-changes.md) made a confirmed edit
materialize its line changes, so `orders` and `order_items` describe the same
purchase. It explicitly left inventory movement for a second step. Three gaps
remained:

- The confirmation guard only looked at after-sales documents. A line with two
  shipped units out of four could be edited down to one, and `fulfill`'s
  `saturating_sub` would then silently hide the missing remaining quantity.
- The edit changed the order total without recording the money difference or
  offering a way to collect or refund it. The order and the payment ledger
  drifted apart.
- Existing shipments were not reconciled with the new line quantities. An order
  that was `fulfilled` and then gained a unit stayed `fulfilled`, and `fulfill`
  refused to ship the unit because it only accepts confirmed or processing
  orders.

Medusa's Order module solves the same three sides with OrderChange,
OrderTransaction/outstanding amount, and draft orders. Its workflows also do not
auto-charge: they compute what is outstanding and leave collection or refund to
an explicit call. fastcart's claim and exchange settlement already follows that
shape, so the edit flow joins it rather than inventing a second one.

## Decision

`confirm_order_edit` is one `BEGIN IMMEDIATE` transaction and settles all three
sides:

- **Quantity guard.** An edit may not reduce a line below the units already
  recorded in `order_fulfillment_items`. The guard re-reads shipped quantities
  inside the transaction, so a fulfillment that commits between prepare and
  confirm is seen. Shrinking below shipped units is a `409`: the merchant has to
  record a return instead.
- **Stock.** The same transaction applies one signed movement per edit line
  (`reason = 'order_edit'`): added or grown lines consume stock, shrunk or
  removed lines return it. A partial unique index on the movement reference
  (`order-edit:{order_id}:{edit_id}:{edit_item_id}`) makes a replay a no-op.
  Demand is summed per inventory level before any write, and the whole batch
  fails if one level cannot cover it. A reduction may only restock units the
  order actually took: committed reservations plus earlier edit movements, minus
  earlier edit restocks, so an edit on an order placed outside checkout cannot
  invent stock.
- **Money.** Confirmation records `order_edits.difference_total`, measured
  against the order total that was current when the confirmation transaction
  began. It does not capture or refund anything. Collection and refund stay
  explicit, idempotent admin calls keyed by the edit, like the claim and
  exchange differences.
- **Read side.** `payment_summary` projects `total`, `captured_total`,
  `refunded_total`, `net_paid` and `outstanding` from the order and payment
  ledgers. The refund ceiling is captured money, not the order total, because a
  cheaper edit can lower the total below what the customer paid.
- **Fulfillment state.** The confirmation re-derives `fulfillment_status` from
  the new line quantities and shipped units, and moves the order back to
  `processing` when units remain unshipped, so added units are fulfillable.

Order totals and adjustments are read inside the transaction rather than from
the pre-transaction snapshot; a stale draft therefore settles only its own
difference even when another change landed between prepare and confirm.

## Consequences

- Orders are a fully settled document after an edit: lines, totals, stock,
  fulfillment state and the settled difference agree.
- Nothing charges the customer automatically. A dearer edit leaves a positive
  outstanding amount and a collectable session; a cheaper one leaves a refund
  due that the merchant pays explicitly.
- Both refund ledgers are written: the payment provider refund first, then the
  order refund capped by captured money. `refund_with_idempotency` and the order
  refund key keep retries single-shot.
- Admin still exposes only fulfil/refund/inspect, so the new endpoints are API
  first. The Admin order workspace and SDK wrappers are the next step.
- Draft orders remain unimplemented; the PRD keeps them as an open question.
