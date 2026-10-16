package kinds

// An interface embedding another: its method set includes Reader.Read.
type ReadCloser interface {
	Reader
	Close()
}
