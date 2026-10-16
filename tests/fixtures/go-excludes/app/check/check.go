// Package check is test support: it imports "testing".
package check

import "testing"

func Equal(t *testing.T, a, b string) {
	if a != b {
		t.Fatal(a, b)
	}
}
