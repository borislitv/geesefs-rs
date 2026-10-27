"""Explicit xattr and forced-attribute workload using Python and GNU coreutils.

Standard S3 listings do not carry user metadata. This additional workload asks
both mounts for it through xattrs and revalidates kernel attributes with GNU
stat. It does not replace the ordinary list/stat workload or change mount caches.
"""
from concurrent.futures import ThreadPoolExecutor
import os
import resource
import shutil
import subprocess
import time


def metadata_roundtrip(root, manifest):
    begun = time.monotonic()
    def attributes(entry):
        errors = []
        for field in ("mode", "mtime", "uid", "gid"):
            expected = str(entry[field] | 0o100000 if field == "mode" else entry[field]).encode()
            try:
                observed = os.getxattr(root / entry["path"], "user." + field)
                if observed != expected:
                    errors.append({"path": entry["path"], "xattr": field,
                                   "expected": expected.decode(), "observed": observed.decode(errors="replace")})
            except OSError as error:
                errors.append({"path": entry["path"], "xattr": field, "error": str(error)})
        return errors
    with ThreadPoolExecutor(max_workers=4) as pool:
        errors = [error for result in pool.map(attributes, manifest) for error in result]
    # A GETXATTR can update daemon metadata while an earlier LOOKUP's attributes
    # remain cached in the kernel. FORCE_SYNC requests new attributes; it does not
    # turn off either mount's metadata or payload cache.
    program = shutil.which("gnustat") or shutil.which("stat")
    result = subprocess.run([program, "--cached=never", "--printf=%s %a %Y %u %g\n", "--",
                             *(str(root / entry["path"]) for entry in manifest)],
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=120)
    if result.returncode:
        errors.append({"stat_exitcode": result.returncode, "stderr": result.stderr})
    rows = result.stdout.splitlines()
    if len(rows) != len(manifest):
        errors.append({"stat_rows": len(rows), "expected": len(manifest)})
    for entry, row in zip(manifest, rows):
        size, mode, mtime, uid, gid = row.split()
        observed = (int(size), int(mode, 8), int(mtime), int(uid), int(gid))
        expected = tuple(entry[field] for field in ("size", "mode", "mtime", "uid", "gid"))
        if observed != expected:
            errors.append({"path": entry["path"], "stat_expected": expected, "stat_observed": observed})
    return {"checked_files": len(manifest), "xattr_checks": len(manifest) * 4, "errors": errors,
            "seconds": time.monotonic() - begun, "stat_stdout": result.stdout,
            "stat_subprocess_maxrss_bytes": resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss * 1024,
            "stat_rss_scope": "Linux waited-child maximum, separate from Python client RSS"}
