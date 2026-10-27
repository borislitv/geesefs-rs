import os
from pathlib import Path
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest import mock

from accounting import CpuWindow, cpu_efficiency


class CpuAccountingTests(unittest.TestCase):
    def test_counter_deltas_separate_daemon_client_cgroup_and_throttling(self):
        with tempfile.TemporaryDirectory() as temporary:
            group = Path(temporary)
            (group / "cpu.stat").write_text("usage_usec 1000000\nuser_usec 800000\nsystem_usec 200000\nnr_periods 2\nnr_throttled 1\nthrottled_usec 10000\n")
            clock = iter((1_000_000_000, 3_000_000_000))
            process = mock.Mock()
            process.create_time.return_value = 42
            process.cpu_times.side_effect = [SimpleNamespace(user=1, system=.2), SimpleNamespace(user=2, system=.5),
                                             SimpleNamespace(user=1.5, system=.3), SimpleNamespace(user=2.1, system=.55)]
            with mock.patch("accounting.psutil.Process", return_value=process):
                window = CpuWindow({"daemon": 123, "client": 456}, group, clock_ns=lambda: next(clock)).start()
                (group / "cpu.stat").write_text("usage_usec 2200000\nuser_usec 1900000\nsystem_usec 300000\nnr_periods 7\nnr_throttled 3\nthrottled_usec 40000\n")
                result = cpu_efficiency(window.finish().metrics(), transferred=1024**3, operations=3)
            for key, expected in {"cpu_wall_seconds": 2, "daemon_cpu_user_seconds": .5,
                "daemon_cpu_system_seconds": .1, "daemon_cpu_seconds": .6, "client_cpu_seconds": .15,
                "cgroup_cpu_seconds": 1.2, "cgroup_cpu_user_seconds": 1.1, "cgroup_cpu_system_seconds": .1,
                "daemon_average_cores": .3, "cgroup_average_cores": .6, "cgroup_throttled_seconds": .03,
                "cgroup_nr_periods": 5, "cgroup_nr_throttled": 2, "daemon_cpu_seconds_per_gib": .6,
                "cgroup_cpu_seconds_per_operation": .4}.items():
                self.assertAlmostEqual(result[key], expected, msg=key)

    def test_pid_reuse_never_looks_like_a_lower_cpu_result(self):
        process = mock.Mock()
        process.create_time.side_effect = [10, 20]
        process.cpu_times.return_value = SimpleNamespace(user=1, system=.2)
        with mock.patch("accounting.psutil.Process", return_value=process):
            window = CpuWindow({"daemon": 123}).start().finish()
            with self.assertRaisesRegex(RuntimeError, "identity changed"):
                window.metrics()

    def test_missing_cgroup_and_zero_byte_denominators_are_missing_not_zero(self):
        window = CpuWindow({"daemon": os.getpid()}).start().finish()
        result = cpu_efficiency(window.metrics(), transferred=0, operations=0)
        self.assertIsNone(result["cgroup_cpu_seconds"])
        self.assertIsNone(result["cgroup_throttled_seconds"])
        self.assertIsNone(result["daemon_cpu_seconds_per_gib"])
        self.assertIsNone(result["daemon_cpu_seconds_per_operation"])

    def test_reset_cgroup_counters_reject_instead_of_clamping_to_zero(self):
        with tempfile.TemporaryDirectory() as temporary:
            group = Path(temporary)
            path = group / "cpu.stat"
            path.write_text("usage_usec 100\nuser_usec 80\nsystem_usec 20\n")
            window = CpuWindow({"daemon": os.getpid()}, group).start()
            path.write_text("usage_usec 10\nuser_usec 8\nsystem_usec 2\n")
            with self.assertRaisesRegex(RuntimeError, "counters decreased"):
                window.finish().metrics()

    def test_real_process_cpu_increases_during_actual_cpu_work(self):
        window = CpuWindow({"daemon": os.getpid()}).start()
        until = time.process_time() + .04
        while time.process_time() < until:
            sum(i * i for i in range(500))
        result = window.finish().metrics()
        self.assertGreater(result["daemon_cpu_seconds"], .01)
        self.assertGreater(result["daemon_average_cores"], 0)
