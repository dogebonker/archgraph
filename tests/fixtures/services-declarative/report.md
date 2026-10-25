# Declared service architecture

4 services, 5 declared calls, 1 cycle (largest: 2 services), 1 hub, 1 forbidden call.

> Only declared calls are analyzed; this graph does not claim completeness.

> Source locations and confidence are supplied by the manifest and are not verified.

> A service cycle is a review signal, not proof of deadlock or deploy-order failure; queue calls may be asynchronous.

## Cycles

- billing, frontend-bff — high confidence; calls #1, #2.

## Hubs

- db-service: called by 2 services — billing, frontend-bff.

## Services

| Service | Fan-in | Fan-out | Instability |
| --- | ---: | ---: | ---: |
| billing | 1 | 3 | 0.7500 |
| db-service | 2 | 0 | 0.0000 |
| frontend-bff | 1 | 2 | 0.6667 |
| notifications | 1 | 0 | 0.0000 |

## Declared calls

Call numbers below are one-based; JSON edge indices are zero-based. Allowed means no declared forbid rule matched.

- #1 frontend-bff → billing — http; high confidence; apps/frontend/src/invoices.ts:18; POST /invoices; allowed.
- #2 billing → frontend-bff — grpc; high confidence; services/billing/customer.go:42; Frontend.GetCustomer; allowed.
- #3 billing → notifications — queue; medium confidence; services/billing/events.py:27; invoice.created; allowed.
- #4 frontend-bff → db-service — http; high confidence; apps/frontend/src/customer.ts:9; GET /customers; forbidden by bff-must-use-service-api (Use the billing service API instead of accessing storage directly.).
- #5 billing → db-service — grpc; high confidence; services/billing/database.go:14; Database.GetCustomer; allowed.
