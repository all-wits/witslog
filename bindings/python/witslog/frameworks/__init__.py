"""Optional web-framework adapters. Each is import-guarded so the core has no
framework dependency — import the one you use:

    from witslog.frameworks.fastapi import add_witslog
    from witslog.frameworks.flask import Witslog
    # Django: add "witslog.frameworks.django.WitslogMiddleware" to MIDDLEWARE
"""
