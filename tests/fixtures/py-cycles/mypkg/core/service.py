from mypkg.db import repo


def serve() -> str:
    return repo.find()
