package domain

import "example.com/layers/adapters/postgres"

type Order struct{ ID string }

// The domain layer reaching into an adapter: what the layers rule forbids.
func Save(o Order) { postgres.Insert(o.ID) }
