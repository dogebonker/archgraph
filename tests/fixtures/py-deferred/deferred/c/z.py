from deferred.b.y import Y


class Z(Y):
    def helper(self) -> object:
        import deferred.d.w

        return deferred.d.w.W()
