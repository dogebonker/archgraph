package app

import "example.com/lib"

type Runner struct{}

func Run() string { return lib.Name() + wired() }
