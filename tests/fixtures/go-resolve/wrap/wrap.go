package wrap

import "github.com/external/missing"

// Do comes from a module outside the repository, which is never loaded: its type is unknown.
var Do = missing.Do
