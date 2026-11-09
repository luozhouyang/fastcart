# Module data ownership

Fastcart is a modular monolith: every commerce module has its own crate, but
all store modules share one SQLite database. Cargo dependencies do not enforce
data ownership, so this document is the reviewed contract and
`crates/api/tests/module_boundaries.rs` is the CI guard.

## Policy

- A module owns the tables it creates and migrates. It may read and write those
  tables directly.
- Cross-module **writes** must go through the owning module's service or an
  orchestrator (`crates/workflows`, HTTP routes) that composes both modules.
- Cross-module **reads** must go through the owning service unless the read is
  part of a validated invariant (for example checkout snapshotting a cart, or a
  referential guard). Every exception is listed in `READ_ALLOWED` in the guard
  test with a reason; new direct reads fail CI. Prefer service calls for
  anything that grows.
- Shared infrastructure tables (`settings`, `jobs`, `outbox`,
  `audit_outbox`, `idempotency_keys`, `workflow_executions`, `workflow_steps`)
  are not owned by a commerce module and may be written by their infrastructure
  or maintenance code.
- Do not add a foreign key from one module's table to another module's table.
  Store the identifier as `TEXT` and validate it in the owning service.

## Ownership map

| Tables                                                                   | Owner           |
| ------------------------------------------------------------------------ | --------------- |
| `carts`, `cart_*`                                                        | `cart`          |
| `checkout_sessions`, `checkout_state_history`                            | `checkout`      |
| `orders`, `order_*`, `after_sales_*`                                     | `order`         |
| `products`, `product_*`, `categories`, `collections`, `tags`             | `catalog`       |
| `prices`, `price_*`                                                      | `pricing`       |
| `inventory_*`                                                            | `inventory`     |
| `customers`, `customer_*`                                                | `customer`      |
| `promotions`, `promotion_*`, `coupons`, `campaign*`                      | `promotion`     |
| `payments`, `payment_*`, `refunds`                                       | `payment`       |
| `shipping_*`, `fulfillment_*`                                            | `shipping`      |
| `tax_*`                                                                  | `tax`           |
| `themes`, `theme_*`, `preview_token_revocations`                         | `theme`         |
| `pages`, `page_versions`, `navigations`, `navigation_items`, `redirects` | `content`       |
| `inquiries`, `inquiry_*`                                                 | `inquiry`       |
| `assets`, `asset_blob_purges`                                            | `asset`         |
| `notification*`                                                          | `notifications` |
| `webhook_*`                                                              | `webhook`       |
| `settings`, `sales_channels`, `regions`, `publishable_api_keys`          | `store`         |

## Current reviewed exceptions

The guard records writes in `ALLOWED` and reads in `READ_ALLOWED`, each with a
reason. They are debt, not a license for new violations.

### Writes

None. The former order→inventory and pricing→catalog link writes now go through
`InventoryService::reverse_return_write_off` and
`CatalogService::attach_price_set` respectively.

### Reads

| Module     | Table                       | Why                                                               |
| ---------- | --------------------------- | ----------------------------------------------------------------- |
| `catalog`  | `sales_channels`            | validates a sales-channel id against the store table              |
| `catalog`  | `cart_items`, `order_items` | best-seller reporting joins across commerce rows                  |
| `checkout` | `carts`, `cart_items`       | snapshots the cart before creating an order                       |
| `checkout` | `payment_sessions`          | reads payment session state for checkout                          |
| `pricing`  | `customer_groups`           | resolves customer-group membership                                |
| `pricing`  | `sales_channels`            | validates a sales-channel id                                      |
| `pricing`  | `product_variant_price_set` | checks the catalog-owned link inside pricing's own transaction    |
| `tax`      | `product_variants`          | tax-category deletion checks variant references                   |
| `order`    | `refunds`                   | refund reconciliation reads provider refunds to repair the ledger |

Adding an exception requires a reviewer and a concrete removal plan; the
`allowed_exceptions_still_exist` and `read_exceptions_still_exist` tests reject
stale entries.
