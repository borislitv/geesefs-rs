"""Independent fixture boundary, never folded into daemon/client target metrics."""
import json
from pathlib import Path
import threading
import time

from accounting import CpuWindow
from compare import SampledBytes, proc_memory


def process_io(pid):
    return {key.rstrip(':'): int(value) for key, value in
            (line.split() for line in Path(f'/proc/{pid}/io').read_text().splitlines())}


class FixtureSampler:
    def __init__(self, processes, group, trace):
        self.processes, self.group, self.trace = dict(processes), Path(group), trace
        self.cpu = CpuWindow({'provider': processes['provider']}, self.group)
        self.done, self.failure = threading.Event(), None
        self.thread = threading.Thread(target=self.run)

    def run(self):
        try:
            with self.trace.open('w') as target:
                while not self.done.is_set():
                    rss = {role: proc_memory(pid).get('rss') for role, pid in self.processes.items()}
                    if any(value is None for value in rss.values()):
                        raise RuntimeError('owned fixture process RSS unavailable')
                    target.write(json.dumps(dict(monotonic_ns=time.monotonic_ns(),
                        provider=rss['provider'], process_rss=sum(rss.values()),
                        cgroup_memory=int((self.group / 'memory.current').read_text()))) + '\n')
                    target.flush()
                    self.done.wait(0.02)
        except Exception as error:
            self.failure = error

    def __enter__(self):
        self.before_io = {role: process_io(pid) for role, pid in self.processes.items()}
        self.started_ns = time.monotonic_ns()
        self.cpu.start()
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.finished_ns = time.monotonic_ns()
        self.done.set()
        try:
            self.cpu.finish()
            self.after_io = {role: process_io(pid) for role, pid in self.processes.items()}
        finally:
            self.thread.join(timeout=10)
        if self.thread.is_alive() or self.failure:
            raise RuntimeError(f'fixture accounting failed: {self.failure}')

    def metrics(self):
        series = {name: SampledBytes() for name in ['provider', 'process_rss', 'cgroup_memory']}
        midpoint = (self.started_ns + self.finished_ns) // 2
        with self.trace.open() as source:
            for line in source:
                row = json.loads(line)
                if self.started_ns <= row['monotonic_ns'] <= self.finished_ns:
                    for name, samples in series.items():
                        samples.record(row[name], late=row['monotonic_ns'] >= midpoint)
        io = {role: {name: self.after_io[role][name] - value for name, value in before.items()}
              for role, before in self.before_io.items()}
        if any(value < 0 for values in io.values() for value in values.values()):
            raise RuntimeError('fixture process I/O counters decreased')
        return dict(cpu=self.cpu.metrics(), cpu_counters=dict(before=self.cpu.before, after=self.cpu.after),
            memory={name: dict(peak=value.peak, latter_half_mean=value.late_mean) for name, value in series.items()},
            process_io_delta=io, scope='provider + supervisor + daemon + workload client RSS; whole owned ancestor cgroup CPU/memory; per-process /proc I/O, not hardware IOPS',
            rss_scope='20ms simultaneous samples, not sum of independent peaks; steady is temporal latter-half mean',
            cgroup_peak_bytes=int((self.group / 'memory.peak').read_text()),
            cgroup_peak_scope='owned-fixture lifetime including seeding/preparation and all preceding cases')
