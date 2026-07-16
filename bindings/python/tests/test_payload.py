"""Pure marshalling tests — no native library required."""

import pytest

import witslog


def test_payload_has_required_fields():
    p = witslog._build_payload("app", "msg")
    assert p == {"application": "app", "message": "msg"}


def test_payload_passes_context_tags_metadata_through():
    p = witslog._build_payload(
        "app",
        "msg",
        context={"request_id": "r1"},
        tags=["a", "b"],
        metadata={"k": "v"},
        severity="warn",
    )
    assert p["context"] == {"request_id": "r1"}
    assert p["tags"] == ["a", "b"]
    assert p["metadata"] == {"k": "v"}
    assert p["severity"] == "warn"


def test_payload_drops_none_fields():
    p = witslog._build_payload("app", "msg", category=None, tags=None)
    assert "category" not in p and "tags" not in p


def test_payload_rejects_unknown_field():
    with pytest.raises(ValueError):
        witslog._build_payload("app", "msg", bogus="x")


def test_payload_rejects_non_str_application():
    with pytest.raises(TypeError):
        witslog._build_payload(123, "msg")


def test_payload_rejects_non_str_message():
    with pytest.raises(TypeError):
        witslog._build_payload("app", 123)


def test_payload_rejects_non_list_tags():
    with pytest.raises(TypeError):
        witslog._build_payload("app", "msg", tags="not-a-list")


def test_encode_is_utf8_bytes():
    raw = witslog._encode({"application": "app", "message": "héllo"})
    assert isinstance(raw, bytes)
    assert "héllo".encode("utf-8") in raw
