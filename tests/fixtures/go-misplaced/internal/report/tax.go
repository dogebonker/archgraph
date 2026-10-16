package report

// Only invoice uses Tax, but it calls round, unexported in util.go: moved out of package report
// it would not compile, so it is not proposed.
func Tax(cents int) int { return round(cents / 10) }
