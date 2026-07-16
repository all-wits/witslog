"""FastAPI / Starlette adapter.

    from fastapi import FastAPI
    from witslog.frameworks.fastapi import add_witslog

    app = FastAPI()
    add_witslog(app, application="myapi", config={"buffer": {"enabled": True}})

Mounts witslog on startup, flushes on shutdown, and logs any unhandled exception
raised while serving a request (then re-raises so FastAPI's own handling runs).
"""

import witslog


def add_witslog(app, application="fastapi", config=None):
    """Wire witslog into a FastAPI/Starlette `app`. Returns `app` for chaining."""

    @app.on_event("startup")
    def _witslog_startup():  # pragma: no cover - requires a running server
        witslog.init(config)

    @app.on_event("shutdown")
    def _witslog_shutdown():  # pragma: no cover - requires a running server
        witslog.shutdown()

    try:
        from starlette.middleware.base import BaseHTTPMiddleware
    except ImportError as exc:  # pragma: no cover
        raise ImportError("add_witslog requires FastAPI/Starlette installed") from exc

    class _WitslogMiddleware(BaseHTTPMiddleware):
        async def dispatch(self, request, call_next):
            try:
                return await call_next(request)
            except Exception as exc:  # noqa: BLE001 - log then re-raise
                witslog.exception(
                    application,
                    exc,
                    context={"path": str(request.url.path), "method": request.method},
                )
                raise

    app.add_middleware(_WitslogMiddleware)
    return app
