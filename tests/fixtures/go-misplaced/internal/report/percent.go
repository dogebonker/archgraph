package report

// A method of Rate, declared in rate.go: Go keeps both in package report, so neither file is
// proposed for a move although only invoice uses them.
func (r Rate) Percent(cents int) int { return cents * 100 / r.Basis }
