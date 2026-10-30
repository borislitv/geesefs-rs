#!/usr/bin/env python3
"""Disposable loopback Moto S3 with deterministic application-level faults.

Werkzeug owns HTTP; Moto owns S3/signature/storage behavior. No production proxy
or custom S3 protocol is implemented here. All in-memory state dies on exit.
"""
import argparse
import json
import logging
import threading
from urllib.parse import parse_qs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, required=True)
    args = parser.parse_args()
    from moto.server import create_backend_app
    from werkzeug.serving import make_server

    backend = create_backend_app("s3")
    faults = set()
    lock = threading.Lock()

    def app(environ, start_response):
        path = environ.get("PATH_INFO", "")
        method = environ.get("REQUEST_METHOD", "")
        query = parse_qs(environ.get("QUERY_STRING", ""))
        if path == "/_geesefs_test_fault":
            with lock:
                faults.clear()
                faults.update(query.get("mode", []))
            start_response("200 OK", [("Content-Type", "application/json")])
            return [json.dumps({"faults": sorted(faults)}).encode()]
        if path.endswith("/fault"):
            with lock:
                fail = (
                    "put" in faults and method == "PUT"
                    or "copy" in faults and "HTTP_X_AMZ_COPY_SOURCE" in environ
                    or "complete" in faults and method == "POST" and "uploadId" in query
                )
            if fail:
                # Completion's embedded HTTP-200 error is a real S3 failure mode.
                status = "200 OK" if method == "POST" else "500 Internal Server Error"
                start_response(status, [("Content-Type", "application/xml")])
                return [b"<Error><Code>InternalError</Code><Message>disposable injected fault</Message></Error>"]
        return backend(environ, start_response)

    logging.getLogger("werkzeug").setLevel(logging.ERROR)
    make_server("127.0.0.1", args.port, app, threaded=True).serve_forever()


if __name__ == "__main__":
    main()
