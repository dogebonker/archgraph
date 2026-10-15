from app.adapters.repository import save
from app.service_layer.services import create


def handle() -> str:
    return create() + save()
