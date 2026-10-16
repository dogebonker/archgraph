package sql

import "example.com/cycles/api/types"

func Insert(o types.Order) string { return o.ID }
