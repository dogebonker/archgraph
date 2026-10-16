package store

type DB struct{ values map[string]string }

func (d *DB) Get(k string) string { return d.values[k] }
