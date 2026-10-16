//go:build !windows

// The other platforms' probe: the same function as probe_windows.go, left out with it.
package app

func probe(t any) {}
