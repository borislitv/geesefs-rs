#!/usr/bin/env python3
"""aiohttp owns full/Range HTTP semantics for the mounted-file client test."""
import argparse
from pathlib import Path
from aiohttp import web

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--file", type=Path, required=True)
parser.add_argument("--port", type=int, required=True)
args = parser.parse_args()


async def payload(_request):
    return web.FileResponse(args.file)


app = web.Application()
app.router.add_get("/payload", payload)
web.run_app(app, host="127.0.0.1", port=args.port, print=None)
