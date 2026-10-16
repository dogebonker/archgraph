package store

func Open() *DB { return &DB{values: map[string]string{}} }
