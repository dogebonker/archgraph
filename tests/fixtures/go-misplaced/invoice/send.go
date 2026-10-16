package invoice

import (
	"example.com/misplaced/internal/report"
	"example.com/misplaced/pub"
)

func Send() string {
	return report.Amount(pub.Money(report.Tax(report.Default.Percent(100)))) + report.Notify() + report.Label(1)
}
