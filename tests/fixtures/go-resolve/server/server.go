package server

import "example.com/resolve/store"

type Server struct {
	base
	db *store.DB
}

func New() *Server { return &Server{db: store.Open()} }
