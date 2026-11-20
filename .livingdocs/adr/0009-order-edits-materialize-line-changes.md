---
type: ADR
title: Order edits must materialize their line changes
description: Confirming an order edit must materialize its line changes, so the order totals and order_items always describe the same purchase.
owner: luozhouyang
status: Accepted
implementation: complete
timestamp: 2026-11-20T00:00:00Z
---

# 0009. Order edits must materialize their line changes

## Context

`create_order` enforces an identity between an order's stored totals and its
lines: `subtotal` is the sum of line subtotals, `discount_total` the sum of line
discounts, and `total` is the sum of line totals plus shipping plus order-level
adjustments (`resolve_order` in `crates/modules/order/src/service.rs`).

`confirm_order_edit` broke that identity. It scaled `orders` and
`order_adjustments` by `retained_subtotal / subtotal` and confirmed the
`order_edit_items` draft, but left `order_items` untouched — deliberately, as
the module documented "historical `order_items` rows are never updated".

The result was an order whose aggregate described the edited purchase while its
lines still described the original one. Every consumer that reads lines —
fulfillment's remaining shippable quantity, return/claim/exchange availability,
invoices, reports — then operates on quantities the totals no longer match. A
line removed by an edit can still be returned in full.

## Decision

Confirming an edit must materialize its line changes, so the `resolve_order`
identity holds after every edit:

- An added line is inserted into `order_items`.
- A quantity change updates that line's `quantity` and recomputes its
  `subtotal`, `discount_total`, `tax_total` and `total`.
- A removal sets the line's quantity to zero. Line rows are never deleted:
  `order_return_items`, `order_claim_items` and `order_exchange_items`
  reference `order_items.id` with `ON DELETE RESTRICT`.
- Order totals are then derived from the lines rather than scaled
  independently, which is the same identity checkout already relies on.
- A tax line follows the base it is levied on: an item-scoped tax line shrinks
  with its line, a shipping-scoped one is preserved with the shipping, and
  `orders.tax_total` becomes the sum of those tax lines.
- Two guards are added: a confirmation fails when a line's quantity no longer
  matches the edit's `before_quantity` (someone else changed it while the draft
  was open), and an edit cannot shrink a line below the quantity already
  committed to after-sales documents.

## Status

Accepted and implemented. `confirm_order_edit` materializes the draft on
`order_items` and derives the order totals, including `tax_total`, from those
rows and the scaled tax lines. The acceptance tests in
`crates/modules/order/tests/order_edit_line_consistency.rs` pass.

The tax-line rounding question is settled without placing a remainder on a
chosen line: because `orders.tax_total` *is* the sum of the scaled tax lines,
the identity `Σ order_tax_lines == orders.tax_total` that `create_order`
enforces cannot drift, whatever per-line flooring does. The rule is covered by
`order_tax_lines_follow_their_base_and_keep_adding_up` in the service tests,
including an order whose tax is partly levied on shipping.

Inventory movement for edits is deliberately out of scope here and should be a
second step once line changes exist to reference.

## Consequences

- `order.items` is the current state of the order rather than an immutable
  record of the original purchase. The module docs, the `after_sales` module
  docs and the domain docs now say so. The header of
  `migrations/store/0018_returns.sql` still states the opposite; a migration
  cannot be edited in place, so that wording has to be corrected separately.
- Auditing "what did the customer originally buy" moves to `order_edits` and
  `order_edit_items`, which record `before_quantity`/`after_quantity`.
