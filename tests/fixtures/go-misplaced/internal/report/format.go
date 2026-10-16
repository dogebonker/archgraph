package report

import "fmt"

func Amount(cents int) string { return fmt.Sprintf("%d.%02d", cents/100, cents%100) }
