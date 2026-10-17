"""Linux process-tree observations, explicitly distinct from cgroup accounting."""

from __future__ import annotations

import os
import threading
import time
from pathlib import Path


def cpu_set(value: str) -> set[int]:
    cpus = set()
    for group in value.split(","):
        ends = group.split("-")
        if len(ends) == 1:
            cpus.add(int(ends[0]))
        elif len(ends) == 2:
            cpus.update(range(int(ends[0]), int(ends[1]) + 1))
        else:
            raise ValueError(f"invalid CPU set: {value}")
    if not cpus or min(cpus) < 0:
        raise ValueError("CPU set must contain nonnegative logical CPU IDs")
    return cpus


def read_process(pid: int) -> dict | None:
    try:
        text = Path(f"/proc/{pid}/stat").read_text()
        # comm can contain spaces and parentheses. Fields after the final ')'
        # begin with state (field 3); own CPU excludes waited-child CPU.
        fields = text[text.rfind(")") + 2:].split()
        return {
            "pid": pid, "ppid": int(fields[1]), "pgrp": int(fields[2]),
            "comm": text[text.find("(") + 1:text.rfind(")")], "state": fields[0],
            "ticks": int(fields[11]) + int(fields[12]),
            "waited_child_ticks": int(fields[13]) + int(fields[14]),
            "threads": int(fields[17]), "start_ticks": int(fields[19]),
            "rss_bytes": int(fields[21]) * os.sysconf("SC_PAGE_SIZE"),
        }
    except (OSError, ValueError, IndexError):
        return None


class TreeMonitor:
    """Sample descendants and process groups, retaining PID/start-time identity.

    CPU from sampled PIDs is a lower bound: a very short child may be missed.
    Sum RSS counts shared mappings more than once; it is not cgroup memory/PSS.
    No sampled measure is presented as a hard memory-limit enforcement proof.
    """

    def __init__(self, allowed_cpus: set[int], interval: float = 0.1):
        self.allowed_cpus = allowed_cpus
        self.interval = interval
        self.roots: set[int] = set()
        self.lock = threading.Lock()
        self.done = threading.Event()
        self.thread = threading.Thread(target=self._loop, daemon=True)
        self.available = Path("/proc/self/stat").exists()
        self.last_ticks: dict[tuple[int, int], int] = {}
        self.processes: dict[tuple[int, int], dict] = {}
        self.peak_rss = 0
        self.peak_threads = 0
        self.samples = 0
        self.affinity_checks = 0
        self.affinity_escapes: set[tuple[int, str]] = set()
        self.started = time.monotonic()

    def add(self, pid: int) -> None:
        with self.lock:
            self.roots.add(pid)

    def start(self) -> None:
        self.thread.start()

    def _sample(self) -> None:
        if not self.available:
            return
        with self.lock:
            roots = self.roots.copy()
        if not roots:
            return
        all_rows = {}
        try:
            entries = os.scandir("/proc")
            with entries:
                for entry in entries:
                    if entry.name.isdigit():
                        row = read_process(int(entry.name))
                        if row:
                            all_rows[row["pid"]] = row
        except OSError:
            self.available = False
            return
        included = set(roots)
        previous = -1
        while len(included) != previous:
            previous = len(included)
            included.update(pid for pid, row in all_rows.items()
                            if row["ppid"] in included or row["pgrp"] in roots)
        rows = [all_rows[pid] for pid in included if pid in all_rows]
        self.peak_rss = max(self.peak_rss, sum(row["rss_bytes"] for row in rows))
        self.peak_threads = max(self.peak_threads, sum(row["threads"] for row in rows))
        for row in rows:
            key = (row["pid"], row["start_ticks"])
            self.last_ticks[key] = max(self.last_ticks.get(key, 0), row["ticks"])
            self.processes[key] = row
            try:
                for task in Path(f"/proc/{row['pid']}/task").iterdir():
                    try:
                        affinity = os.sched_getaffinity(int(task.name))
                        self.affinity_checks += 1
                        if not affinity <= self.allowed_cpus:
                            self.affinity_escapes.add((int(task.name), ",".join(map(str, sorted(affinity)))))
                    except (ProcessLookupError, PermissionError):
                        continue
            except (FileNotFoundError, PermissionError):
                continue
        self.samples += 1

    def _loop(self) -> None:
        while not self.done.is_set():
            self._sample()
            self.done.wait(self.interval)

    def snapshot(self) -> dict:
        return {
            "proc_available": self.available,
            "sample_interval_seconds": self.interval,
            "samples": self.samples,
            "observed_cpu_seconds_lower_bound": sum(self.last_ticks.values()) / os.sysconf("SC_CLK_TCK"),
            "peak_sum_rss_bytes": self.peak_rss,
            "peak_thread_count": self.peak_threads,
            "observed_process_count": len(self.processes),
            "thread_affinity_checks": self.affinity_checks,
            "affinity_escapes": [list(row) for row in sorted(self.affinity_escapes)],
            "processes": list(self.processes.values()),
            "limitations": [
                "sampled process CPU may miss short-lived child processes",
                "sum RSS double-counts shared mappings and may miss peaks between samples",
                "affinity checks are sampled, not continuous; cgroup limits are a separate control",
            ],
        }

    def finish(self) -> dict:
        self.done.set()
        self.thread.join(timeout=2)
        self._sample()
        return self.snapshot()
