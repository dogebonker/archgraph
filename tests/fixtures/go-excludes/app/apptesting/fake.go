// Package apptesting holds fakes for tests: test code by its name.
package apptesting

import "example.com/excludes/app"

func Fake() string { return app.Run() }
