package handler

import "example.com/cycles/store"

type Handler struct{}

// Serve and route refer to each other's files: one package, so no file cycle.
func (h Handler) Serve() { route(); store.Save() }
