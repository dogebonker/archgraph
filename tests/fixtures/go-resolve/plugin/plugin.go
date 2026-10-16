// Package plugin registers the store by importing it for its side effects.
package plugin

import (
	_ "example.com/resolve/blank"
	_ "example.com/resolve/missing"
	_ "example.com/resolve/store"
)
