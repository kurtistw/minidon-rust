"""Harness wrapper for the Python oracle. Run from the oracle copy, never from the reference repo.

It starts the unmodified `minidon.main:app` with uvicorn (as `python -m minidon` does, but on a
chosen port) and freezes the clock (review M-05).

Clock: `minidon.db.utcnow` is the one function every module uses (`accounts/service.py` imports
it by name, `accounts/models.py` binds it as a column default). It reads the module global
`datetime` of `minidon.db` on every call, so replacing that one global freezes every use. No
reference file is changed.

Control: `POST /__clock` with {"at": "2026-01-01T00:00:00"} sets the time. Not a Minidon route.
Modes: `--create-schema` does what `python -m minidon` does first (`create_tables()`) and exits.
"""

import sys
from datetime import datetime as real_datetime

import minidon.db as db

_state = {"now": real_datetime(2026, 1, 1, 0, 0, 0)}


class FrozenDatetime(real_datetime):
    @classmethod
    def now(cls, tz=None):  # noqa: ARG003 - the reference calls now(UTC) and drops the zone
        at = _state["now"]
        return at.replace(tzinfo=tz) if tz is not None else at


db.datetime = FrozenDatetime


def main() -> None:
    if sys.argv[1] == "--create-schema":
        db.create_tables()
        return

    import uvicorn
    from fastapi import Request
    from fastapi.responses import JSONResponse

    from minidon.main import app

    @app.post("/__clock")
    async def set_clock(request: Request) -> JSONResponse:
        body = await request.json()
        _state["now"] = real_datetime.fromisoformat(body["at"])
        return JSONResponse({"now": _state["now"].isoformat()})

    uvicorn.run(app, host="127.0.0.1", port=int(sys.argv[1]), log_level="warning")


if __name__ == "__main__":
    main()
