package report

type Summary struct{ Total int }

func (s Summary) Text() string { return "total" }
