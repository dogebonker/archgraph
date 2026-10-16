package billing

import "example.com/internals/billing/ledger"

func Charge(cents int) { ledger.Record(cents) }
