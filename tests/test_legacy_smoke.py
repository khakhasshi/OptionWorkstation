from fastapi.testclient import TestClient

from backend.app import app


def test_catalog_is_available_without_licensed_market_data() -> None:
    response = TestClient(app).get("/api/catalog")

    assert response.status_code == 200
    assert response.json()["engine"] == "python-reference"
    assert isinstance(response.json()["symbols"], list)
