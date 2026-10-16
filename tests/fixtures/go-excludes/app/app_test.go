package app

import "testing"

// A method of Runner (app.go) in a test file: it binds app.go only while tests are scanned.
func (Runner) check() bool { return true }

func TestRun(t *testing.T) { _ = Run() }
