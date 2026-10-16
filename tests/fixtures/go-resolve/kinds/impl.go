package kinds

type File struct{}

func (File) Read() int { return 1 }
