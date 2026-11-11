"""Public SDK Range contract/cost probe; no FUSE and no provider internals.

All reads stream in bounded chunks. Provider /proc rchar is logical read syscall
bytes, NOT disk traffic; read_bytes is storage I/O charged by Linux.
"""
import argparse
import contextlib
import json
from pathlib import Path
import sys
import tempfile
import threading
import time
import urllib.request

import psutil
from botocore.exceptions import ClientError

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import integration

MIB = 1024 ** 2


def snapshot(pid):
    process = psutil.Process(pid)
    cpu, io = process.cpu_times(), process.io_counters()
    return dict(user=cpu.user, system=cpu.system, rss=process.memory_info().rss,
                rchar=io.read_chars, read_bytes=io.read_bytes)


def heap_metrics(url):
    if url is None:
        return None
    with urllib.request.urlopen(url, timeout=5) as response:
        text = response.read(8 * MIB + 1)
    if len(text) > 8 * MIB:
        raise RuntimeError('provider metrics exceeded bounded response')
    names = ['go_memstats_heap_alloc_bytes', 'go_memstats_alloc_bytes_total', 'go_memstats_heap_sys_bytes']
    values = {}
    for line in text.decode().splitlines():
        fields = line.split()
        if len(fields) == 2 and fields[0] in names:
            values[fields[0]] = float(fields[1])
    return values


def measure(pid, action, metrics_url=None):
    heap_before = heap_metrics(metrics_url)
    before = snapshot(pid)
    peak = [before['rss']]
    stop = threading.Event()

    def sample():
        while not stop.wait(0.005):
            peak[0] = max(peak[0], snapshot(pid)['rss'])

    sampler = threading.Thread(target=sample)
    sampler.start()
    started = time.monotonic()
    try:
        result = action()
    finally:
        stop.set()
        sampler.join(timeout=5)
    after = snapshot(pid)
    elapsed = time.monotonic() - started
    return dict(result=result, seconds=elapsed, before=before, after=after,
                heap_before=heap_before, heap_after=heap_metrics(metrics_url),
                rss_peak=peak[0], rss_increase=max(0, peak[0] - before['rss']),
                delta={key: after[key] - before[key] for key in ['user', 'system', 'rchar', 'read_bytes']})


def expect_error(call, status):
    try:
        result = call()
    except ClientError as error:
        assert error.response['ResponseMetadata']['HTTPStatusCode'] == status, error.response
        return
    if 'Body' in result:
        result['Body'].close()
    raise AssertionError(f'provider admitted request requiring HTTP {status}')


def read_exact(s3, bucket, key, etag, offset, length, size, whole=False):
    options = {} if whole else {'Range': f'bytes={offset}-{offset + length - 1}'}
    response = s3.get_object(Bucket=bucket, Key=key, IfMatch=etag, **options)
    with contextlib.closing(response['Body']) as body:
        assert response['ResponseMetadata']['HTTPStatusCode'] == (200 if whole else 206)
        assert response['ContentLength'] == length
        if not whole:
            assert response['ContentRange'] == f'bytes {offset}-{offset + length - 1}/{size}'
        count = 0
        while data := body.read(128 * 1024):
            assert data == integration.expected_range(offset + count, len(data)), 'Range bytes mismatch'
            count += len(data)
        assert count == length, (count, length)
    return length


def probe(s3, bucket, pid, size=128 * MIB, metrics_url=None):
    digest = integration.seed(s3, bucket, 'probe', size, 1)
    key = 'probe/payload'
    head = s3.head_object(Bucket=bucket, Key=key)
    assert head['ContentLength'] == size
    etag = head['ETag']
    expect_error(lambda: s3.get_object(Bucket=bucket, Key=key, Range='bytes=0-131071', IfMatch='"wrong"'), 412)
    expect_error(lambda: s3.get_object(Bucket=bucket, Key=key, Range=f'bytes={size}-', IfMatch=etag), 416)
    read_exact(s3, bucket, key, etag, size - 17, 17, size)
    response = s3.get_object(Bucket=bucket, Key=key, Range=f'bytes={size - 2}-{size + 7}', IfMatch=etag)
    with contextlib.closing(response['Body']) as body:
        assert response['ResponseMetadata']['HTTPStatusCode'] == 206
        assert response['ContentLength'] == 2
        assert response['ContentRange'] == f'bytes {size - 2}-{size - 1}/{size}'
        assert body.read() == integration.expected_range(size - 2, 2), 'clipped EOF bytes'
    rows = {}
    for label, step, total in [('small_ranges', 128 * 1024, 8 * MIB),
                               ('large_range', 8 * MIB, 8 * MIB),
                               ('whole_object', size, size)]:
        def action():
            return sum(read_exact(s3, bucket, key, etag, start, min(step, total - start), size,
                                  whole=label == 'whole_object') for start in range(0, total, step))
        rows[label] = measure(pid, action, metrics_url)
    return dict(size=size, sha256=digest, etag=etag, contracts=['exact Range bytes/status/length',
                'IfMatch 412', 'EOF 416', 'EOF tail', 'clipped EOF 206', 'whole object'], measurements=rows)


def assert_bounded(result):
    small = result['measurements']['small_ranges']
    assert small['delta']['rchar'] < 32 * small['result'], 'provider reads whole object for small Ranges'
    assert small['rss_increase'] < 32 * MIB, 'small Range allocates object-sized body'
    if small.get('heap_before') and small.get('heap_after'):
        allocated = (small['heap_after']['go_memstats_alloc_bytes_total']
                     - small['heap_before']['go_memstats_alloc_bytes_total'])
        assert 0 <= allocated < 32 * MIB, 'small Ranges allocate object-sized Go bodies despite reused RSS'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--weed', type=Path)
    parser.add_argument('--require-bounded', action='store_true')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(dir=args.output, prefix='work-') as temporary:
        work = Path(temporary)
        if args.weed:
            from sealed_s3_fixture import native_service
            service = native_service(args.weed, work)
        else:
            service = integration.service(None, work)
        with service as (s3, bucket, endpoint, _, _):
            if args.weed:
                pid = int((work / 'backend.pid').read_text())
            else:
                children = psutil.Process().children()
                pid = next(p.pid for p in children if any('s3_fixture.py' in a for a in p.cmdline()))
            receipt = json.loads((work / 'provider.json').read_text()) if args.weed else None
            metrics_url = f"http://127.0.0.1:{receipt['ports']['metricsPort']}/metrics" if receipt else None
            result = probe(s3, bucket, pid, metrics_url=metrics_url)
            result.update(provider='SeaweedFS 4.51' if args.weed else 'Moto 5.1.17', endpoint=endpoint,
                          provider_receipt=receipt,
                          memory_scope='sampled provider RSS, not allocator ownership; rchar includes cache reads')
            (args.output / 'report.json').write_text(json.dumps(result, indent=2) + '\n')
            print(json.dumps(result, indent=2), flush=True)
            if args.require_bounded:
                assert_bounded(result)


if __name__ == '__main__':
    main()
