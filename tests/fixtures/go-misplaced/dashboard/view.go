package dashboard

import (
	"example.com/misplaced/internal/report"
	"example.com/misplaced/pub"
)

func View() string { return report.Summary{Total: pub.Rates()}.Text() }
