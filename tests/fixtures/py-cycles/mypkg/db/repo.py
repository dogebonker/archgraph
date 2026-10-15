import mypkg.core
from mypkg.db.session import connect


def find() -> str:
    return connect() + mypkg.core.boot()
