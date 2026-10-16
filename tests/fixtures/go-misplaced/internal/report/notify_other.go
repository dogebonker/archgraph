//go:build !linux && !windows

// Notify for every other platform: the three files are one function, bound together.
package report

func Notify() string { return "other" }
