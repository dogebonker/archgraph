package app

import "testing"

// Production code that also holds a stub for tests: app/cmd uses Finder, so the file stays.
func Finder() string { return "finder" }

func StubFinder(t *testing.T) { t.Helper() }
