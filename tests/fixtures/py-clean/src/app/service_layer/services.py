from app.domain.model import User


def create() -> str:
    return User.name
