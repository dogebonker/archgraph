from app.domain.model import User


def save() -> str:
    return User.name
