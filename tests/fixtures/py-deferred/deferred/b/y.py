from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from deferred.a.x import X


class Y:
    def owner(self) -> "X":
        raise NotImplementedError

    def load(self) -> object:
        from deferred.c.z import Z

        return Z()
