package kinds

type Box[T any] struct{ V T }

func (b Box[T]) Get() T { return b.V }

func Map[T any](x T) T { return x }
