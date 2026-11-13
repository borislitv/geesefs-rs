"""Checked, bounded real-read capacity preparation, not a cache insertion hook.

The controller releases one positioned read at a time and may drain actual
optional demotions between reads. Read-ahead, jobs and resource budgets are not
changed. The distinct pressure identity prevents refreshing the target witness.
"""
import json
import os
from pathlib import Path
import random
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import integration

BLOCK = 128 * 1024
MAX_DATASET = 2 * 1024**3
SEED = 1729
SECONDS = 180


def protocol(size):
    offsets(size, SEED)  # Validate before constructing a worker or reading data.
    return dict(rounds=1, size=size, seed=SEED, block_bytes=BLOCK, seconds=SECONDS,
                relative_path='capacity-pressure/payload', measured_relative_path='capacity-pressure/payload',
                pacing='drain real demotions between positioned reads')


def populate(path, size, environment, group, log, drain):
    """One fixed pass in every arm; no success-based retry or early completion.

    Candidate drain waits for real optional work; disabled arms have no optional
    work to drain. Both execute the identical offsets, reads and byte checks.
    All pacing and process cleanup belong outside the performance interval.
    """
    from compare import read_message
    declaration = protocol(size)
    started = time.monotonic()
    deadline = started + SECONDS
    with log.open('w') as errors:
        process = subprocess.Popen([sys.executable, str(Path(__file__).resolve())],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors, text=True,
            env=environment, preexec_fn=lambda: integration.enter_cgroup(group))
        try:
            spec = dict(path=str(path / declaration['relative_path']), size=size, rounds=1, seed=SEED)
            process.stdin.write(json.dumps(spec) + '\n')
            process.stdin.flush()
            if read_message(process, 10) != {'ready_round': 0}:
                raise RuntimeError('capacity worker did not become ready')
            process.stdin.write('next\n')
            process.stdin.flush()
            consumed = 0
            for expected_offset in offsets(size, SEED):
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise RuntimeError('fixed capacity preparation deadline exceeded')
                message = read_message(process, min(30, remaining))
                if message != {'offset': expected_offset, 'bytes': BLOCK}:
                    raise RuntimeError(f'capacity preparation read failed: {message}')
                consumed += BLOCK
                drain()
                if time.monotonic() >= deadline:
                    raise RuntimeError('fixed capacity preparation deadline exceeded during drain')
                process.stdin.write('next\n')
                process.stdin.flush()
            if read_message(process, 5) != {'round_complete': 0, 'bytes': size}:
                raise RuntimeError('capacity preparation did not complete its fixed pass')
            process.stdin.close()
            process.wait(timeout=5)
            if process.returncode:
                raise RuntimeError(f'capacity worker exited {process.returncode}')
            return dict(protocol=declaration, bytes=consumed, reads=size // BLOCK,
                        byte_correct=True, elapsed_seconds=time.monotonic() - started)
        finally:
            if not process.stdin.closed:
                process.stdin.close()
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            process.stdout.close()


def offsets(size, seed, block=BLOCK):
    if type(block) is not int or block != BLOCK:
        raise ValueError('pressure block must be exactly 128 KiB')
    if type(size) is not int or not 0 < size <= MAX_DATASET or size % block:
        raise ValueError('pressure size must be block-aligned, positive and at most 2 GiB')
    indices = list(range(size // block))
    random.Random(seed).shuffle(indices)
    return (index * block for index in indices)


def worker_main():
    try:
        spec = json.loads(sys.stdin.readline())
        path, size = Path(spec['path']), spec['size']
        # Both leaf names matter: reading the target payload instead of a
        # distinct identity would keep its recent-index hint hot indefinitely.
        if path.name != 'payload' or path.parent.name != 'capacity-pressure':
            raise ValueError('capacity preparation requires distinct capacity-pressure/payload')
        if path.stat().st_size != size:
            raise AssertionError('capacity-pressure object size mismatch')
        if type(spec['rounds']) is not int or not 1 <= spec['rounds'] <= 4 or size * spec['rounds'] > MAX_DATASET:
            raise ValueError('capacity preparation exceeds four rounds or 2 GiB read bound')
        with path.open('rb', buffering=0) as source:
            for number in range(spec['rounds']):
                sequence = offsets(size, spec['seed'] + number)
                print(json.dumps({'ready_round': number}), flush=True)
                command = sys.stdin.readline().strip()
                if command == 'stop':
                    return 0
                if command != 'next':
                    raise RuntimeError('capacity controller ended before round')
                count = 0
                for offset in sequence:
                    data = os.pread(source.fileno(), BLOCK, offset)
                    if len(data) != BLOCK or data != integration.expected_range(offset, BLOCK):
                        raise AssertionError(f'capacity-pressure bytes mismatch at {offset}')
                    count += len(data)
                    print(json.dumps({'offset': offset, 'bytes': len(data)}), flush=True)
                    command = sys.stdin.readline().strip()
                    if command == 'stop':
                        return 0  # Controller deliberately stops only at its own bound.
                    if command != 'next':
                        raise RuntimeError('capacity controller ended during round')
                print(json.dumps({'round_complete': number, 'bytes': count}), flush=True)
        return 0
    except Exception as error:
        print(json.dumps({'error': str(error)}), flush=True)
        return 1


if __name__ == '__main__':
    raise SystemExit(worker_main())
