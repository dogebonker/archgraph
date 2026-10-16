package pub

// Only invoice uses Money, but pub is outside internal/: other modules may import it, so it is
// public API and not proposed for a move.
func Money(cents int) int { return cents }
