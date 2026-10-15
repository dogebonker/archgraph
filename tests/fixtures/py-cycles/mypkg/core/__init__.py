from mypkg.db.session import connect


def boot() -> str:
    return connect()
