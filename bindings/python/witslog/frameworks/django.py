"""Django adapter.

settings.py:

    MIDDLEWARE = [
        ...,
        "witslog.frameworks.django.WitslogMiddleware",
    ]
    WITSLOG_APPLICATION = "mysite"          # optional
    WITSLOG_CONFIG = {"buffer": {"enabled": True}}  # optional

The middleware mounts witslog once (on construction) and logs any exception that
propagates out of a view via `process_exception` (Django then renders its 500).
"""

import witslog


class WitslogMiddleware:
    def __init__(self, get_response):
        self.get_response = get_response
        try:
            from django.conf import settings

            self.application = getattr(settings, "WITSLOG_APPLICATION", "django")
            config = getattr(settings, "WITSLOG_CONFIG", None)
        except Exception:  # noqa: BLE001 - settings not configured (e.g. in tests)
            self.application = "django"
            config = None
        witslog.init(config)

    def __call__(self, request):
        return self.get_response(request)

    def process_exception(self, request, exception):
        witslog.exception(
            self.application,
            exception,
            context={"path": request.path, "method": request.method},
        )
        return None  # let Django continue its own exception handling
