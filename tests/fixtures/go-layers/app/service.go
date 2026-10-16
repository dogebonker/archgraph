package app

import "example.com/layers/domain"

func Place() { domain.Save(domain.Order{ID: "1"}) }
