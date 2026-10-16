package server

import "strings"

func (s *Server) Handle(k string) string { return strings.ToUpper(s.db.Get(k)) }
