---
type: ADR
title: Order edits must materialize their line changes
description: Confirming an order edit rewrites the order totals but leaves order_items frozen, so the aggregate stops describing the purchase.
owner: luozhouyang
status: Proposed
implementation: pending
timestamp: 2026-11-20T00:00:00Z
---

# 0009. Order edits must materialize their line changes

## Context

`create_order` enforces an identity between an order's stored totals and its
lines: `subtotal` is the sum of line subtotals, `discount_total` the sum of line
discounts, and `total` is the sum of line totals plus shipping plus order-level
adjustments (`resolve_order` in `crates/modules/order/src/service.rs`).

`confirm_order_edit` breaks that identity. It scales `orders` and
`order_adjustments` by `retained_subtotal / subtotal` and confirms the
`order_edit_items` draft, but leaves `order_items` untouched — deliberately, as
the module documents "historical `order_items` rows are never updated".

The result is an order whose aggregate describes the edited purchase while its
lines still describe the original one. Every consumer that reads lines —
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
- Two guards are added: a confirmation fails when a line's quantity no longer
  matches the edit's `before_quantity` (someone else changed it while the draft
  was open), and an edit cannot shrink a line below the quantity already
  committed to after-sales documents.

## Status

Proposed. The acceptance tests are checked in as
`crates/modules/order/tests/order_edit_line_consistency.rs` and are `#[ignore]`d
until this lands.

A verified implementation exists as a patch (applied to
`service_after_sales.rs`): with it, all seven acceptance tests pass. It is not
landed because it is a behaviour change to order history that also rewrites
four existing assertions encoding the current "lines are frozen" contract, and
because one question is still open: for an order whose tax is partly levied on
shipping, the per-line tax scaling can leave `Σ order_tax_lines` a few minor
units away from `orders.tax_total`, and where that remainder should land has not
been decided.

Inventory movement for edits is deliberately out of scope here and should be a
second step once line changes exist to reference.

## Consequences

- `order.items` becomes the current state of the order rather than an immutable
  record of the original purchase. The module docs, the `after_sales` module
  docs and the header of `migrations/store/0018_returns.sql` all state the
  opposite and must be updated with the implementation.
- Auditing "what did the customer originally buy" moves to `order_edits` and
  `order_edit_items`, which already record `before_quantity`/`after_quantity`.
- Until this lands, order edits produce orders whose totals and lines disagree.
