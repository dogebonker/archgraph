package report

// Only invoice uses Label, but it works with summary.go, in its own package: a library piece
// the package offers, not code that belongs to invoice.
func Label(cents int) string { return Summary{Total: cents}.Text() }
