package main

import (
	"fmt"

	"example.com/resolve/server"
)

func main() {
	s := server.New()
	fmt.Println(s.Name(), s.Handle("x"))
}
