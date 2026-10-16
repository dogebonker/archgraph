package lost

import (
	"example.com/resolve/kinds"
	"example.com/resolve/wrap"
)

// wrap.Do's type is unknown, so the type checker records nothing inside the call: the use of
// kinds.Map is lost (counted as unresolved), not guessed.
var X = wrap.Do(kinds.Map(1))
