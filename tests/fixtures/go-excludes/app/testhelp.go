package app

import "testing"

// A helper for tests in a file that is not a _test.go file: left out as test support, alone.
func MustRun(t testing.TB) {
	if Run() == "" {
		t.Fatal("empty")
	}
}
