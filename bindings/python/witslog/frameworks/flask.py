"""Flask adapter.

    from flask import Flask
    from witslog.frameworks.flask import Witslog

    app = Flask(__name__)
    Witslog(app, application="myflask", config={"buffer": {"enabled": True}})

Mounts witslog on init, logs unhandled request exceptions via the
`got_request_exception` signal, and flushes on app teardown.
"""

import witslog


class Witslog:
    def __init__(self, app=None, application="flask", config=None):
        self.application = application
        self.config = config
        if app is not None:
            self.init_app(app)

    def init_app(self, app):
        witslog.init(self.config)

        try:
            from flask import got_request_exception, request

            def _log_exception(sender, exception, **extra):
                ctx = {}
                try:
                    ctx = {"path": request.path, "method": request.method}
                except Exception:  # noqa: BLE001 - outside a request context
                    pass
                witslog.exception(self.application, exception, context=ctx)

            got_request_exception.connect(_log_exception, app)
        except ImportError as exc:  # pragma: no cover
            raise ImportError("Witslog(app) requires Flask installed") from exc

        @app.teardown_appcontext
        def _flush(exc):  # noqa: ARG001
            witslog.flush()
