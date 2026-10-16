//go:build ignore

// A generator: never part of a build, so not a file of the graph.
package main

import "example.com/resolve/store"

func main() { store.Open() }
