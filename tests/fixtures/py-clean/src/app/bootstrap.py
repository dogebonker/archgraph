from app.adapters.repository import save
from app.entrypoints.api import handle


def boot() -> str:
    return handle() + save()
