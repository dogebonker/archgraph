package app

import "testing"

// Production code asking whether it runs under go test: testing.Testing alone is no test
// support, so this file stays. It also calls probe, declared once per platform.
func InTest() bool {
	probe(nil)
	return testing.Testing()
}
