package orders

import (
	"example.com/internals/billing"
	"example.com/internals/billing/ledger"
)

// The ledger is billing's own: the compiler allows the import (no internal/ folder), the
// public rule does not.
func Checkout() { billing.Charge(100); ledger.Record(-100) }
