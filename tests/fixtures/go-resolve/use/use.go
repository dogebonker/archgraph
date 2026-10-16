package use

import "example.com/resolve/kinds"

// r.Read() is Reader's method (iface.go), rc.Read() the same method through the embedding,
// b.Get() the generic method (gen.go), kinds.F the alias (alias.go), not File.
func Use(r kinds.Reader, rc kinds.ReadCloser) int {
	var b kinds.Box[int]
	var a kinds.F
	_ = a
	return r.Read() + rc.Read() + b.Get() + kinds.Map(1)
}
