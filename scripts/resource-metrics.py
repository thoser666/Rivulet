#!/usr/bin/env python3
"""Drive `rivulet metrics` and gate its numbers against a committed baseline.

Issue #363 (M9): measure the real application instead of a synthetic fixture.

What this script does, and why each part is where it is:

1. It spawns the shipped `rivulet` binary with `rivulet metrics ...`, which
   drives the real engine bootstrap, a real idle window, the real headless
   render path and optionally a real recording session.
2. It samples the child process **from outside** (CPU time, RSS, thread
   count). A process reporting its own footprint can be wrong about it; an
   external sampler cannot. Phase markers on the child's stdout say which
   window a sample belongs to.
3. It reduces the samples to the resource-efficiency JSON schema, then gates
   the numbers against a committed baseline per profile.
4. Where a number cannot be measured honestly (no GPU, no battery on a shared
   runner, no engine because GStreamer is missing), the field is recorded as
   `n/a`/`blocked` **with the reason**. A fabricated pass is the failure mode
   this script exists to prevent.

The doc (`docs/resource-efficiency-goal.md`) is the source of the targets; the
thresholds here are the machine-checkable subset of them. Everything that
needs a game, a real capture backend or laptop hardware stays manual evidence.

Usage:

    python3 scripts/resource-metrics.py --self-test
    python3 scripts/resource-metrics.py --profile ci-linux --out report.json
    python3 scripts/resource-metrics.py --profile ci-linux --write-baseline
"""

import argparse
import json
import math
import os
import queue
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path

SCHEMA_VERSION = 1

REPO_ROOT = Path(__file__).resolve().parent.parent

#: Where per-profile baselines live. A baseline update is a deliberate commit,
#: never something CI writes on its own.
BASELINE_DIR = REPO_ROOT / "scripts" / "resource-metrics-baseline"

#: The scene the preview phase renders. Not the render-smoke card: that fixture
#: proves byte-identical output on a 64x48 canvas, where per-frame cost is
#: indistinguishable from scheduling noise. This one is 720p so the compositor
#: has real work to do and a regression in the render path moves the number.
DEFAULT_CONFIG = REPO_ROOT / "scripts" / "ci-resource-metrics" / "preview.toml"

#: Minimum idle window. The goal doc says "60-second idle sample"; anything
#: shorter is not the target being measured.
MIN_IDLE_SECONDS = 5.0

#: Samples the first `settle` seconds of the idle window are dropped: startup
#: work (allocator warmup, plugin registry, thread pool creation) happens there
#: and would be attributed to idling.
DEFAULT_SETTLE_SECONDS = 2.0

#: How often the sampler reads the process. Ten samples per second is enough to
#: catch a spinning thread without making the sampler the workload.
SAMPLE_INTERVAL_SECONDS = 0.25

#: The gate. `absolute` is added to the baseline; `factor` multiplies it. A
#: budget is breached when the measurement exceeds **both** limits, so a tiny
#: baseline cannot turn scheduler noise into a failure and a real regression
#: still fails by a wide margin.
#:
#: - idle CPU: the goal doc's "<= 2 % additional process CPU after startup
#:   settles". Absolute, because the target is absolute.
#: - RSS slope: no unbounded growth. 4 MiB/min over a minute is ~4 MiB of
#:   drift, far above a flat heap and far below a real leak.
#: - memory growth: the existing 64 MiB session budget.
#: - startup: 3x the baseline **and** +500 ms. Shared runners are noisy; a
#:   regression worth failing on is not 200 ms.
#: - p99 frame time: 2x **and** +20 ms, for the same reason -- the headless
#:   compositor on a shared runner is not a frame-pacing environment.
BUDGETS = {
    "idle_cpu_percent": {"absolute": 2.0},
    "rss_slope_mb_per_min": {"absolute": 4.0},
    "memory_growth_mb": {"absolute": 64.0},
    "startup_ms": {"factor": 3.0, "absolute": 500.0},
    "p99_frame_time_ms": {"factor": 2.0, "absolute": 20.0},
}

#: Targets the goal doc states as absolute, independent of any baseline. They
#: cap the relative allowance: a baseline of 2.4 % idle CPU must not buy a
#: 4.4 % budget, because 2 % is the documented target. The schema gate
#: (`scripts/resource-efficiency-check.py`) enforces the same numbers, so the
#: two layers cannot disagree.
ABSOLUTE_MAX = {
    "idle_cpu_percent": 2.0,
    "rss_slope_mb_per_min": 4.0,
    "memory_growth_mb": 64.0,
}

#: Metrics whose number is only meaningful relative to its own baseline.
RELATIVE_TO_BASELINE = ("startup_ms", "p99_frame_time_ms")

PASS = "pass"
FAIL = "fail"
NA = "n/a"
BLOCKED = "blocked"


class MetricsError(RuntimeError):
    """A measurement could not be produced at all (no report is written)."""


# --------------------------------------------------------------------------
# Sampling the child process
# --------------------------------------------------------------------------


@dataclass
class Sample:
    """One reading of the measured process."""

    wall: float
    cpu_seconds: float
    rss_mb: float
    threads: int | None
    phase: str | None


class ProcessSampler:
    """Reads CPU time, RSS and thread count of a process, per platform.

    `Unavailable` is raised when the platform cannot answer; callers turn that
    into a `blocked`/`n/a` verdict with the reason instead of a number.
    """

    class Unavailable(RuntimeError):
        pass

    def __init__(self, pid):
        self.pid = pid
        self._platform = sys.platform
        self._handle = None
        self._reason = None
        if self._platform == "win32":
            self._open_windows_handle()
        elif self._platform.startswith("linux"):
            if not Path("/proc").is_dir():
                self._reason = "no /proc on this system"
        elif self._platform == "darwin":
            if shutil.which("ps") is None:
                self._reason = "no `ps` binary on this system"
        else:
            self._reason = f"no sampler for platform {self._platform!r}"

    # -- platform setup --------------------------------------------------

    def _open_windows_handle(self):
        import ctypes
        from ctypes import wintypes

        PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        handle = kernel32.OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION, False, wintypes.DWORD(self.pid)
        )
        if not handle:
            self._reason = (
                f"OpenProcess failed (error {ctypes.get_last_error()})"
            )
            return
        self._handle = handle
        self._kernel32 = kernel32
        self._wintypes = wintypes
        self._ctypes = ctypes

    # -- public API ------------------------------------------------------

    def sample(self, phase=None):
        if self._reason is not None:
            raise self.Unavailable(self._reason)
        if self._platform == "win32":
            cpu, threads = self._sample_windows()
            return Sample(time.monotonic(), cpu, self._rss_windows(), threads, phase)
        if self._platform.startswith("linux"):
            cpu, threads = _linux_cpu_and_threads(self.pid)
            return Sample(time.monotonic(), cpu, _linux_rss_mb(self.pid), threads, phase)
        if self._platform == "darwin":
            cpu, rss, threads = _darwin_sample(self.pid)
            return Sample(time.monotonic(), cpu, rss, threads, phase)
        raise self.Unavailable(self._reason or "unsupported platform")

    def close(self):
        if self._handle:
            self._kernel32.CloseHandle(self._handle)
            self._handle = None

    # -- Windows ---------------------------------------------------------

    def _sample_windows(self):
        ctypes = self._ctypes
        wintypes = self._wintypes

        class FILETIME(ctypes.Structure):
            _fields_ = [("dwLowDateTime", wintypes.DWORD), ("dwHighDateTime", wintypes.DWORD)]

        creation = FILETIME()
        exit_time = FILETIME()
        kernel = FILETIME()
        user = FILETIME()
        ok = self._kernel32.GetProcessTimes(
            self._handle,
            ctypes.byref(creation),
            ctypes.byref(exit_time),
            ctypes.byref(kernel),
            ctypes.byref(user),
        )
        if not ok:
            raise self.Unavailable(
                f"GetProcessTimes failed (error {ctypes.get_last_error()})"
            )
        cpu = (_filetime_seconds(kernel) + _filetime_seconds(user))
        return cpu, _windows_thread_count(self.pid)

    def _rss_windows(self):
        ctypes = self._ctypes
        wintypes = self._wintypes

        class PROCESS_MEMORY_COUNTERS(ctypes.Structure):
            _fields_ = [
                ("cb", wintypes.DWORD),
                ("PageFaultCount", wintypes.DWORD),
                ("PeakWorkingSetSize", ctypes.c_size_t),
                ("WorkingSetSize", ctypes.c_size_t),
                ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
                ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
                ("PagefileUsage", ctypes.c_size_t),
                ("PeakPagefileUsage", ctypes.c_size_t),
            ]

        counters = PROCESS_MEMORY_COUNTERS()
        counters.cb = ctypes.sizeof(PROCESS_MEMORY_COUNTERS)
        psapi = ctypes.WinDLL("psapi", use_last_error=True)
        ok = psapi.GetProcessMemoryInfo(
            self._handle, ctypes.byref(counters), counters.cb
        )
        if not ok:
            raise self.Unavailable(
                f"GetProcessMemoryInfo failed (error {ctypes.get_last_error()})"
            )
        return counters.WorkingSetSize / (1024.0 * 1024.0)


def _filetime_seconds(filetime):
    raw = (filetime.dwHighDateTime << 32) | filetime.dwLowDateTime
    return raw * 1e-7  # 100 ns ticks -> seconds


def _windows_thread_count(pid):
    """Thread count via a toolhelp snapshot. `None` when it cannot be read."""
    try:
        import ctypes
        from ctypes import wintypes

        TH32CS_SNAPTHREAD = 0x00000004
        INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value

        class THREADENTRY32(ctypes.Structure):
            _fields_ = [
                ("dwSize", wintypes.DWORD),
                ("cntUsage", wintypes.DWORD),
                ("th32ThreadID", wintypes.DWORD),
                ("th32OwnerProcessID", wintypes.DWORD),
                ("tpBasePri", wintypes.LONG),
                ("tpDeltaPri", wintypes.LONG),
                ("dwFlags", wintypes.DWORD),
            ]

        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        snapshot = kernel32.CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
        if snapshot in (None, INVALID_HANDLE_VALUE):
            return None
        try:
            entry = THREADENTRY32()
            entry.dwSize = ctypes.sizeof(THREADENTRY32)
            if not kernel32.Thread32First(snapshot, ctypes.byref(entry)):
                return None
            count = 0
            while True:
                if entry.th32OwnerProcessID == pid:
                    count += 1
                if not kernel32.Thread32Next(snapshot, ctypes.byref(entry)):
                    break
            return count
        finally:
            kernel32.CloseHandle(snapshot)
    except Exception:  # pragma: no cover - depends on a live Windows host
        return None


def _linux_cpu_and_threads(pid):
    # Fields after the (comm) field start at field 3. utime/stime are fields
    # 14/15 and num_threads is field 20, so their offsets are 11/12/17. The
    # comm field may contain spaces and parentheses, hence the rfind.
    with open(f"/proc/{pid}/stat", "r", encoding="utf-8") as handle:
        raw = handle.read()
    fields = raw[raw.rfind(")") + 2 :].split()
    ticks = os.sysconf("SC_CLK_TCK")
    return (int(fields[11]) + int(fields[12])) / ticks, int(fields[17])


def _linux_rss_mb(pid):
    with open(f"/proc/{pid}/status", "r", encoding="utf-8") as handle:
        for line in handle:
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) / 1024.0
    raise ProcessSampler.Unavailable("/proc/<pid>/status has no VmRSS")


_PS_TIME = re.compile(r"^(?:(?P<days>\d+)-)?(?:(?P<hours>\d+):)?(?P<minutes>\d+):(?P<seconds>\d+(?:\.\d+)?)$")


def _darwin_sample(pid):
    """CPU seconds, RSS MiB and thread count from `ps` on macOS."""
    completed = subprocess.run(
        ["ps", "-o", "rss=,time=,thcount=", "-p", str(pid)],
        capture_output=True,
        text=True,
        timeout=10,
    )
    if completed.returncode != 0 or not completed.stdout.strip():
        raise ProcessSampler.Unavailable(
            f"ps exited {completed.returncode}: {completed.stderr.strip() or 'no output'}"
        )
    parts = completed.stdout.split()
    rss = int(parts[0]) / 1024.0
    match = _PS_TIME.match(parts[1])
    if not match:
        raise ProcessSampler.Unavailable(f"unrecognised ps time {parts[1]!r}")
    cpu = (
        int(match.group("days") or 0) * 86400
        + int(match.group("hours") or 0) * 3600
        + int(match.group("minutes")) * 60
        + float(match.group("seconds"))
    )
    threads = int(parts[2]) if len(parts) > 2 else None
    return cpu, rss, threads


# --------------------------------------------------------------------------
# Power and thermal telemetry
# --------------------------------------------------------------------------


def collect_power(requested):
    """Battery/thermal evidence, or an honest reason there is none.

    This is never a hard CI threshold on a shared runner: the doc requires the
    number to be *reported*, not invented.
    """
    if not requested:
        return {
            "status": NA,
            "reason": "not requested (pass --power on hardware that has a battery)",
            "samples": [],
        }
    try:
        samples = _power_samples()
    except Exception as error:  # pragma: no cover - platform call
        return {"status": BLOCKED, "reason": f"telemetry unavailable: {error}", "samples": []}
    if not samples:
        return {
            "status": NA,
            "reason": "the OS exposed no battery or thermal reading on this machine",
            "samples": [],
        }
    return {"status": PASS, "reason": "reported, not thresholded", "samples": samples}


def _power_samples():
    if sys.platform.startswith("linux"):
        return _linux_power()
    if sys.platform == "win32":
        return _windows_power()
    if sys.platform == "darwin":
        return _darwin_power()
    return []


def _linux_power():
    samples = []
    battery_dir = Path("/sys/class/power_supply")
    if battery_dir.is_dir():
        for entry in sorted(battery_dir.iterdir()):
            uevent = entry / "uevent"
            if not uevent.is_file():
                continue
            values = {}
            for line in uevent.read_text(encoding="utf-8", errors="replace").splitlines():
                key, _, value = line.partition("=")
                values[key] = value
            if values.get("POWER_SUPPLY_TYPE") != "Battery":
                continue
            reading = {
                "source": f"battery:{entry.name}",
                "on_battery": values.get("POWER_SUPPLY_STATUS") == "Discharging",
                "capacity_percent": _to_float(values.get("POWER_SUPPLY_CAPACITY")),
                "power_watts": _to_float(values.get("POWER_SUPPLY_POWER_NOW"), 1e6),
            }
            samples.append(reading)
    for zone in sorted(Path("/sys/class/thermal").glob("thermal_zone*")):
        temp = zone / "temp"
        if not temp.is_file():
            continue
        try:
            celsius = _to_float(temp.read_text(encoding="utf-8").strip(), 1000.0)
        except OSError:
            continue
        if celsius is None:
            continue
        name = zone / "type"
        samples.append(
            {
                "source": f"thermal:{name.read_text(encoding='utf-8').strip() if name.is_file() else zone.name}",
                "celsius": celsius,
            }
        )
    return samples


def _windows_power():  # pragma: no cover - needs a Windows host
    script = (
        "$b = Get-CimInstance -ClassName Win32_Battery | Select-Object -First 1 "
        "BatteryStatus,EstimatedChargeRemaining; "
        "$t = Get-CimInstance -Namespace root/wmi -ClassName MSAcpi_ThermalZoneTemperature "
        "-ErrorAction SilentlyContinue | Select-Object -First 1 CurrentTemperature; "
        "@{battery=$b; thermal=$t} | ConvertTo-Json -Compress -Depth 3"
    )
    completed = subprocess.run(
        ["powershell", "-NoProfile", "-NonInteractive", "-Command", script],
        capture_output=True,
        text=True,
        timeout=30,
    )
    if completed.returncode != 0 or not completed.stdout.strip():
        return []
    data = json.loads(completed.stdout)
    samples = []
    battery = data.get("battery") or {}
    if battery:
        samples.append(
            {
                "source": "battery:Win32_Battery",
                # 2 == "on AC", 1 == "discharging" in Win32_Battery.
                "on_battery": battery.get("BatteryStatus") == 1,
                "capacity_percent": battery.get("EstimatedChargeRemaining"),
            }
        )
    thermal = data.get("thermal") or {}
    if thermal and thermal.get("CurrentTemperature") is not None:
        samples.append(
            {
                "source": "thermal:MSAcpi_ThermalZoneTemperature",
                "celsius": thermal["CurrentTemperature"] / 10.0 - 273.15,
            }
        )
    return samples


def _darwin_power():  # pragma: no cover - needs a macOS host
    completed = subprocess.run(
        ["pmset", "-g", "batt"], capture_output=True, text=True, timeout=10
    )
    if completed.returncode != 0:
        return []
    text = completed.stdout
    samples = []
    match = re.search(r"(\d+)%;\s*(\w+(?: \w+)?)", text)
    if match:
        samples.append(
            {
                "source": "battery:pmset",
                "on_battery": "discharging" in match.group(2).lower(),
                "capacity_percent": float(match.group(1)),
            }
        )
    match = re.search(r"([\d.]+)\s*degrees? C", text)
    if match:
        samples.append({"source": "thermal:pmset", "celsius": float(match.group(1))})
    return samples


def _to_float(raw, divisor=1.0):
    if raw is None:
        return None
    try:
        return float(raw) / divisor
    except (TypeError, ValueError):
        return None


# --------------------------------------------------------------------------
# Running one measured process
# --------------------------------------------------------------------------


@dataclass
class Phase:
    name: str
    duration_ms: float
    detail: dict = field(default_factory=dict)
    started_at: float | None = None
    ended_at: float | None = None


@dataclass
class MeasuredRun:
    label: str
    command: list
    startup_ms: float
    engine: str
    report: dict
    phases: dict
    samples: list
    stderr_path: Path
    exit_code: int


def run_measured(binary, metrics_args, label, out_dir, timeout):
    """Spawn `rivulet metrics ...`, sample it, and return what was observed."""
    command = [str(binary), "metrics"] + metrics_args
    stderr_path = out_dir / f"{label}.stderr.log"
    with open(stderr_path, "wb") as stderr:
        spawned_at = time.monotonic()
        process = subprocess.Popen(
            command,
            stdout=subprocess.PIPE,
            stderr=stderr,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
        )
        try:
            sampler = ProcessSampler(process.pid)
        except Exception as error:
            process.kill()
            raise MetricsError(f"cannot sample pid {process.pid}: {error}")

        lines: queue.Queue = queue.Queue()

        def reader():
            for line in process.stdout:
                lines.put((time.monotonic(), line.rstrip("\n")))
            lines.put((time.monotonic(), None))

        thread = threading.Thread(target=reader, daemon=True)
        thread.start()

        samples = []
        phases = {}
        current_phase = None
        startup_ms = None
        report = None
        decode_error = None
        last_sample = 0.0

        try:
            while True:
                if time.monotonic() - last_sample >= SAMPLE_INTERVAL_SECONDS:
                    try:
                        samples.append(sampler.sample(current_phase))
                    except ProcessSampler.Unavailable as error:
                        # CPU/RSS are the deliverable, not a nice-to-have: a
                        # report without them would be exactly the "looks fine"
                        # output this gate exists to prevent.
                        process.kill()
                        raise MetricsError(f"{label}: sampling failed ({error})")
                    last_sample = time.monotonic()

                remaining = timeout - (time.monotonic() - spawned_at)
                if remaining <= 0:
                    process.kill()
                    raise MetricsError(
                        f"{label}: exceeded the {timeout:.0f} s measurement window"
                    )
                try:
                    arrived, line = lines.get(timeout=min(SAMPLE_INTERVAL_SECONDS, remaining))
                except queue.Empty:
                    continue
                if line is None:
                    break
                event = _parse_event(line)
                if event is None:
                    continue
                kind = event.get("event")
                if kind == "phase_start":
                    current_phase = event.get("phase")
                    phases.setdefault(
                        current_phase,
                        Phase(
                            name=current_phase,
                            duration_ms=math.nan,
                            started_at=arrived,
                        ),
                    )
                elif kind == "phase_end":
                    phase = phases.setdefault(
                        event.get("phase"),
                        Phase(name=event.get("phase"), duration_ms=math.nan),
                    )
                    phase.ended_at = arrived
                    phase.duration_ms = float(event.get("duration_ms", math.nan))
                    phase.detail = event.get("detail") or {}
                    if phase.name == "startup":
                        startup_ms = (arrived - spawned_at) * 1000.0
                    current_phase = None
                elif kind == "report":
                    report = event
                elif kind == "error":
                    decode_error = event.get("message", "unknown error")

            exit_code = process.wait(timeout=30)
        finally:
            try:
                process.kill()
            except Exception:
                pass
            sampler.close()

    if report is None:
        raise MetricsError(
            f"{label}: `rivulet metrics` produced no report (exit {exit_code}); "
            f"see {stderr_path}"
            + (f" -- {decode_error}" if decode_error else "")
        )

    return MeasuredRun(
        label=label,
        command=command,
        startup_ms=startup_ms if startup_ms is not None else math.nan,
        engine=report.get("engine", "unknown"),
        report=report,
        phases=phases,
        samples=samples,
        stderr_path=stderr_path,
        exit_code=exit_code,
    )


def _parse_event(line):
    line = line.strip()
    if not line.startswith("{"):
        return None
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        return None
    return event if isinstance(event, dict) and "event" in event else None


# --------------------------------------------------------------------------
# Reducing samples to metrics
# --------------------------------------------------------------------------


def idle_window(run, settle_seconds):
    """Samples inside the idle phase, after the settle window."""
    phase = run.phases.get("idle")
    if phase is None or phase.started_at is None or phase.ended_at is None:
        return []
    start = phase.started_at + settle_seconds
    return [sample for sample in run.samples if start <= sample.wall <= phase.ended_at]


def cpu_percent(samples):
    """Process CPU as a percentage of one core over the sampled window."""
    if len(samples) < 2:
        return math.nan
    wall = samples[-1].wall - samples[0].wall
    if wall <= 0:
        return math.nan
    cpu = samples[-1].cpu_seconds - samples[0].cpu_seconds
    return max(0.0, cpu) / wall * 100.0


def rss_slope_mb_per_min(samples):
    """Least-squares slope of RSS over time, in MiB per minute."""
    if len(samples) < 3:
        return math.nan
    times = [sample.wall - samples[0].wall for sample in samples]
    values = [sample.rss_mb for sample in samples]
    mean_t = sum(times) / len(times)
    mean_v = sum(values) / len(values)
    denominator = sum((t - mean_t) ** 2 for t in times)
    if denominator == 0:
        return 0.0
    numerator = sum((t - mean_t) * (v - mean_v) for t, v in zip(times, values))
    return numerator / denominator * 60.0


def phase_cpu_percent(run, name, settle_seconds=0.0):
    phase = run.phases.get(name)
    if phase is None or phase.started_at is None or phase.ended_at is None:
        return math.nan
    start = phase.started_at + settle_seconds
    window = [sample for sample in run.samples if start <= sample.wall <= phase.ended_at]
    return cpu_percent(window)


def frame_metrics(run):
    stats = run.report.get("frame_stats") or {}
    times = run.report.get("frame_times_ms") or []
    mean = sum(times) / len(times) if times else math.nan
    return {
        "mean_frame_time_ms": mean,
        "p50_frame_time_ms": stats.get("p50_ms", math.nan),
        "p95_frame_time_ms": stats.get("p95_ms", math.nan),
        "p99_frame_time_ms": stats.get("p99_ms", math.nan),
        "max_frame_time_ms": stats.get("max_ms", math.nan),
        "one_percent_low_fps": stats.get("one_percent_low_fps"),
        "frames_measured": stats.get("frames", 0),
    }


# --------------------------------------------------------------------------
# Gating against the baseline
# --------------------------------------------------------------------------


def load_baseline(path):
    if path is None or not Path(path).is_file():
        return None
    try:
        data = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise MetricsError(f"{path}: cannot read baseline ({error})")
    if data.get("status") != "measured":
        return None
    return data


def gate(metrics, baseline):
    """Compare measured metrics with a baseline; name metric, baseline and delta."""
    verdicts = []
    baseline_metrics = (baseline or {}).get("metrics") or {}
    for name, budget in BUDGETS.items():
        measured = metrics.get(name)
        if not _is_number(measured):
            verdicts.append(
                {
                    "metric": name,
                    "status": NA,
                    "measured": None,
                    "baseline": baseline_metrics.get(name),
                    "reason": "not measured in this run",
                }
            )
            continue
        base = baseline_metrics.get(name)
        if not _is_number(base):
            verdicts.append(
                {
                    "metric": name,
                    "status": BLOCKED,
                    "measured": measured,
                    "baseline": None,
                    "reason": "no committed baseline for this profile; record one with --write-baseline",
                }
            )
            continue
        allowed = _allowed(name, base, budget)
        delta = measured - base
        status = FAIL if measured > allowed else PASS
        verdicts.append(
            {
                "metric": name,
                "status": status,
                "measured": measured,
                "baseline": base,
                "delta": delta,
                "allowed": allowed,
                "budget": budget,
                "reason": (
                    f"{name} {measured:.2f} exceeds baseline {base:.2f} "
                    f"(delta {delta:+.2f}, allowed up to {allowed:.2f})"
                    if status == FAIL
                    else None
                ),
            }
        )
    return verdicts


def _allowed(name, baseline, budget):
    allowed = baseline + budget.get("absolute", 0.0)
    if "factor" in budget:
        allowed = max(allowed, baseline * budget["factor"])
    if name in ABSOLUTE_MAX:
        allowed = min(allowed, ABSOLUTE_MAX[name])
    return allowed


def _is_number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def overall_status(verdicts):
    if any(verdict["status"] == FAIL for verdict in verdicts):
        return FAIL
    return PASS


def run_status(verdicts, engine, engine_reason, profile):
    """The report's overall status, and the reason when it is not a pass.

    A `blocked` run is not a failure: it means the number could not be produced
    honestly (no engine, no committed baseline yet). It must still never look
    like a pass, which is why an all-blocked run reports `blocked` with the
    reason instead of inheriting `overall_status`'s pass.
    """
    status = overall_status(verdicts)
    reason = None
    if status == FAIL:
        reason = "; ".join(
            verdict["reason"] for verdict in verdicts if verdict["status"] == FAIL
        )
    elif engine != "gstreamer":
        # Without the engine, startup and idle describe a process that never
        # initialised GStreamer.
        status = BLOCKED
        reason = f"engine {engine}: {engine_reason or 'not available'}"
    elif all(verdict["status"] == BLOCKED for verdict in verdicts):
        # No metric was compared with anything, so nothing here may be called a
        # pass. The numbers are still recorded (that is how the first baseline
        # gets measured) and the run does not gate.
        status = BLOCKED
        reason = (
            f"no measured baseline for profile {profile!r}; this run records the "
            "reference numbers and does not gate"
        )
    return status, reason


# --------------------------------------------------------------------------
# The measurement itself
# --------------------------------------------------------------------------


def resolve_binary(requested):
    if requested:
        path = Path(requested)
        if not path.is_file():
            raise MetricsError(f"--binary: {path} does not exist")
        return path
    candidates = [REPO_ROOT / "target" / "debug" / "rivulet", REPO_ROOT / "target" / "debug" / "rivulet.exe"]
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    raise MetricsError(
        "no rivulet binary found; build one first "
        "(cargo build -p rivulet-cli) or pass --binary"
    )


def binary_profile(binary):
    parts = Path(binary).resolve().parts
    return "release" if "release" in parts else "debug"


def baseline_refusal(profile_name, allow_debug):
    """Why a baseline must not be written from this binary, or `None`.

    A baseline is the threshold reference every later run is compared with, so
    it has to describe the binary users run. A debug build is orders of
    magnitude slower in the render path; calibrating on it would either gate
    release runs against numbers that make them trivially pass, or fail every
    debug run against a release baseline.
    """
    if profile_name == "debug" and not allow_debug:
        return (
            "refusing to write a baseline from a debug binary: its numbers are "
            "not what users run. Build with `cargo build --release` and pass "
            "--binary target/release/rivulet, or use --allow-debug-baseline for "
            "a throwaway local profile"
        )
    return None


def measure(args):
    binary = resolve_binary(args.binary)
    config = Path(args.config)
    if not config.is_file():
        raise MetricsError(f"--config: {config} does not exist")

    out_dir = Path(args.out).parent if args.out else REPO_ROOT / "target" / "resource-metrics"
    out_dir.mkdir(parents=True, exist_ok=True)

    idle_seconds = float(args.idle_seconds)
    if idle_seconds < MIN_IDLE_SECONDS:
        raise MetricsError(
            f"--idle-seconds must be at least {MIN_IDLE_SECONDS:g} s: the goal doc's "
            "idle target is a 60-second window, not a snapshot"
        )
    preview = ["--config", str(config), "--frames", str(args.frames)]
    timeout = idle_seconds + float(args.record_seconds) + float(args.timeout_padding)

    idle_run = run_measured(
        binary,
        ["--idle-seconds", f"{idle_seconds:g}"] + preview,
        label="capture-off",
        out_dir=out_dir,
        timeout=timeout,
    )

    ab_run = None
    if args.ab:
        ab_run = run_measured(
            binary,
            ["--record-seconds", f"{idle_seconds:g}"] + preview,
            label="capture-on",
            out_dir=out_dir,
            timeout=timeout,
        )

    window = idle_window(idle_run, float(args.settle_seconds))
    if len(window) < 3:
        raise MetricsError(
            f"only {len(window)} samples landed in the idle window; the sampler "
            f"({sys.platform}) or the settle window ({args.settle_seconds:g} s) is wrong"
        )

    idle_cpu = cpu_percent(window)
    slope = rss_slope_mb_per_min(window)
    growth = max(0.0, window[-1].rss_mb - window[0].rss_mb)
    threads = [sample.threads for sample in window if sample.threads is not None]
    frames = frame_metrics(idle_run)
    if not _is_number(frames["p99_frame_time_ms"]):
        raise MetricsError(
            "the preview phase produced no frame timings, so p99 frame time is "
            "not measurable; pass --config <scene.toml> --frames <n>"
        )

    metrics = {
        "idle_cpu_percent": idle_cpu,
        # Legacy schema field: the additional process CPU while the recorder is
        # idle. Same measurement, named the way the validator asks for it.
        "cpu_delta_percent": idle_cpu,
        "rss_slope_mb_per_min": slope,
        "memory_growth_mb": growth,
        "peak_rss_mb": max(sample.rss_mb for sample in idle_run.samples)
        if idle_run.samples
        else math.nan,
        "threads_max": max(threads) if threads else None,
        "startup_ms": idle_run.startup_ms,
        "engine_init_ms": _phase_duration(idle_run, "startup"),
        **frames,
    }

    engine = idle_run.engine
    engine_reason = _phase_reason(idle_run, "startup")
    if engine != "gstreamer" and args.require_engine:
        raise MetricsError(
            "the engine was not available, so startup and idle are not measured "
            f"honestly ({engine}: {engine_reason}); pass --no-require-engine to "
            "record this as blocked instead"
        )

    capture_ab = {"status": NA, "reason": "not requested (--ab)"}
    if ab_run is not None:
        capture_ab = _capture_ab(idle_run, ab_run, float(args.settle_seconds))
        metrics["capture_cpu_percent"] = capture_ab.get("capture_cpu_percent", math.nan)
        metrics["capture_cpu_overhead_percent"] = capture_ab.get(
            "overhead_percent", math.nan
        )

    power = collect_power(args.power)

    baseline_path = Path(args.baseline) if args.baseline else BASELINE_DIR / f"{args.profile}.json"
    baseline = load_baseline(baseline_path)
    reference = (baseline or {}).get("metrics", {}).get("p99_frame_time_ms")
    if _is_number(reference) and reference > 0:
        metrics["frame_time_regression_percent"] = (
            metrics["p99_frame_time_ms"] - reference
        ) / reference * 100.0
    else:
        # No reference yet, so this run *is* the reference: its delta against
        # it is zero by definition. `frame_time_reference` below says which of
        # the two this is, so the zero is never read as "no drift measured".
        metrics["frame_time_regression_percent"] = 0.0
    frame_time_reference = reference if _is_number(reference) else None

    verdicts = gate(metrics, baseline)
    status, reason = run_status(verdicts, engine, engine_reason, args.profile)

    report = {
        "schema_version": SCHEMA_VERSION,
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "binary": str(binary),
        "baseline": {
            "path": str(baseline_path),
            "status": baseline.get("status") if baseline else "seed",
            "recorded_at": (baseline or {}).get("recorded_at"),
        },
        "profiles": [
            {
                "name": args.profile,
                "platform": _platform_name(),
                "runner": os.environ.get("RUNNER_OS", "local"),
                "binary_profile": binary_profile(binary),
                "engine": engine,
                "engine_reason": engine_reason,
                "idle_seconds": idle_seconds,
                "settle_seconds": float(args.settle_seconds),
                "samples": len(window),
                "preview_config": str(config),
                "preview_frames": args.frames,
                "frame_time_reference": frame_time_reference,
                "frame_time_evidence": (
                    "headless render path (scene model + snapshot + compositor) via "
                    "`rivulet render`'s code; not the GPU preview loop"
                ),
                "capture_ab": capture_ab,
                "gpu": {
                    "status": NA,
                    "reason": (
                        "this harness does not measure GPU utilisation; it needs the "
                        "G5 benchmark on reference hardware"
                    ),
                },
                "power": power,
                "verdicts": verdicts,
                "status": status,
                **({"reason": reason} if reason else {}),
                **{
                    key: value
                    for key, value in metrics.items()
                    if not _is_missing(value)
                },
                "not_measured": sorted(
                    key for key, value in metrics.items() if _is_missing(value)
                ),
            }
        ],
    }
    return report, verdicts, baseline_path, binary


def _phase_duration(run, name):
    phase = run.phases.get(name)
    return phase.duration_ms if phase else math.nan


def _phase_reason(run, name):
    phase = run.phases.get(name)
    return (phase.detail or {}).get("reason") if phase else None


def _capture_ab(capture_off, capture_on, settle_seconds):
    off_cpu = phase_cpu_percent(capture_off, "idle", settle_seconds)
    on_cpu = phase_cpu_percent(capture_on, "record")
    record = capture_on.phases.get("record")
    detail = (record.detail if record else {}) or {}
    status = detail.get("status", "unknown")
    if not (_is_number(off_cpu) and _is_number(on_cpu)):
        return {
            "status": BLOCKED,
            "reason": "the capture-on half produced no measurable CPU window",
            "capture_off_cpu_percent": None,
            "capture_cpu_percent": None,
        }
    overhead = on_cpu - off_cpu
    return {
        "status": PASS if status == "ok" else BLOCKED,
        "reason": None if status == "ok" else detail.get("reason", f"record phase: {status}"),
        "capture_off_cpu_percent": off_cpu,
        "capture_cpu_percent": on_cpu,
        "overhead_percent": overhead,
        "frames_pushed": detail.get("frames_pushed"),
        "output_bytes": detail.get("output_bytes"),
        "evidence": (
            "synthetic RGBA frames pushed through a real recording pipeline; the "
            "game-side frame-time delta in the goal doc needs G5 hardware evidence"
        ),
    }


def _platform_name():
    return {"win32": "windows", "darwin": "macos"}.get(sys.platform, sys.platform)


def _is_nan(value):
    return isinstance(value, float) and math.isnan(value)


def _is_missing(value):
    """A value that must not appear as a measurement: absent or NaN."""
    return value is None or _is_nan(value)


# --------------------------------------------------------------------------
# Output
# --------------------------------------------------------------------------


def write_baseline_bytes(report, profile):
    entry = report["profiles"][0]
    baseline = {
        "schema_version": SCHEMA_VERSION,
        "profile": profile,
        "status": "measured",
        "recorded_at": report["generated_at"],
        "platform": entry["platform"],
        "runner": entry["runner"],
        "binary_profile": entry["binary_profile"],
        # Only the gated metrics: a baseline is a threshold reference, not a
        # second copy of the report.
        "metrics": {
            name: entry[name] for name in BUDGETS if _is_number(entry.get(name))
        },
        "notes": (
            "Committed baseline for scripts/resource-metrics.py. Update it with a "
            "deliberate `--write-baseline` run; CI never writes it."
        ),
    }
    return json.dumps(baseline, indent=2, sort_keys=True) + "\n"


def print_summary(report, verdicts, baseline_path):
    entry = report["profiles"][0]
    print(f"resource metrics: profile {entry['name']} ({entry['platform']}, "
          f"{entry['binary_profile']} binary)")
    print(f"  engine: {entry['engine']}" + (f" ({entry['engine_reason']})" if entry["engine_reason"] else ""))
    if entry.get("binary_profile") == "debug":
        print(
            "  note: debug build -- these numbers are not comparable to a "
            "release baseline (measure `--binary target/release/rivulet`)"
        )
    print(f"  idle window: {entry['idle_seconds']:g} s "
          f"({entry['settle_seconds']:g} s settle, {entry['samples']} samples)")
    for key in (
        "startup_ms",
        "engine_init_ms",
        "idle_cpu_percent",
        "rss_slope_mb_per_min",
        "memory_growth_mb",
        "peak_rss_mb",
        "threads_max",
        "mean_frame_time_ms",
        "p99_frame_time_ms",
        "one_percent_low_fps",
    ):
        value = entry.get(key)
        print(f"  {key}: {'-' if value is None else f'{value:.3f}' if isinstance(value, float) else value}")
    print(f"  power: {entry['power']['status']} ({entry['power']['reason']})")
    print(f"  gpu: {entry['gpu']['status']} ({entry['gpu']['reason']})")
    ab = entry["capture_ab"]
    print(f"  capture A/B: {ab['status']}" + (f" ({ab['reason']})" if ab.get("reason") else ""))
    print(f"  baseline: {baseline_path} ({report['baseline']['status']})")
    if report["baseline"]["status"] == "seed":
        print(
            "  note: no committed baseline gates this profile yet; record one with "
            "--write-baseline and commit it"
        )

    symbol = {PASS: "PASS", FAIL: "FAIL", NA: "N/A ", BLOCKED: "BLOCK"}
    print("  gate:")
    for verdict in verdicts:
        measured = verdict.get("measured")
        shown = f"{measured:.2f}" if _is_number(measured) else "-"
        baseline = verdict.get("baseline")
        shown_base = f"{baseline:.2f}" if _is_number(baseline) else "-"
        line = f"    {symbol[verdict['status']]}  {verdict['metric']:<22} {shown:>10} (baseline {shown_base})"
        if verdict.get("reason") and verdict["status"] != PASS:
            line += f"\n          {verdict['reason']}"
        print(line)
    print(f"  result: {entry['status'].upper()}")


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Measure Rivulet's idle CPU, memory, startup and frame cost "
        "and gate them against a committed baseline (issue #363)."
    )
    parser.add_argument("--profile", default="local", help="baseline profile name (default: local)")
    parser.add_argument("--binary", help="path to the rivulet binary (default: target/debug/rivulet)")
    parser.add_argument("--config", default=str(DEFAULT_CONFIG), help="scene rendered by the preview phase")
    parser.add_argument("--frames", type=int, default=30, help="preview frames (default: 30)")
    parser.add_argument(
        "--idle-seconds",
        type=float,
        default=60.0,
        help="idle window in seconds (default: 60, minimum: 5)",
    )
    parser.add_argument("--settle-seconds", type=float, default=DEFAULT_SETTLE_SECONDS)
    parser.add_argument(
        "--record-seconds",
        type=float,
        default=0.0,
        help="length of the capture-on half; defaults to --idle-seconds when --ab is set",
    )
    parser.add_argument("--ab", action="store_true", help="also run the capture-on/off A/B")
    parser.add_argument("--power", action="store_true", help="collect battery/thermal telemetry")
    parser.add_argument("--out", help="where to write the report JSON")
    parser.add_argument("--baseline", help="baseline file (default: scripts/resource-metrics-baseline/<profile>.json)")
    parser.add_argument(
        "--write-baseline",
        action="store_true",
        help="write this measurement as the profile's baseline (a deliberate commit)",
    )
    parser.add_argument(
        "--allow-debug-baseline",
        action="store_true",
        help="allow --write-baseline from a debug binary (throwaway local profiles only)",
    )
    parser.add_argument(
        "--require-engine",
        dest="require_engine",
        action="store_true",
        default=True,
        help="fail when the engine is unavailable (default: on, for CI)",
    )
    parser.add_argument(
        "--no-require-engine",
        dest="require_engine",
        action="store_false",
        help="record an unavailable engine as blocked instead of failing",
    )
    parser.add_argument("--timeout-padding", type=float, default=120.0)
    parser.add_argument("--json", action="store_true", help="print the report to stdout")
    parser.add_argument("--self-test", action="store_true", help="run the policy self-tests and exit")
    args = parser.parse_args(argv)

    if args.self_test:
        return 0 if run_self_test() else 1

    if args.record_seconds and not args.ab:
        # Only meaningful on its own; the A/B runs it as the second half.
        args.ab = True
    if args.ab and args.record_seconds <= 0:
        args.record_seconds = args.idle_seconds

    try:
        requested_binary = resolve_binary(args.binary)
        if args.write_baseline:
            refusal = baseline_refusal(
                binary_profile(requested_binary), args.allow_debug_baseline
            )
            if refusal is not None:
                raise MetricsError(refusal)
        report, verdicts, baseline_path, binary = measure(args)
    except MetricsError as error:
        print(f"resource metrics: {error}", file=sys.stderr)
        return 1

    if args.out:
        out_path = Path(args.out)
    else:
        out_path = REPO_ROOT / "target" / "resource-metrics" / f"{args.profile}.json"
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    print_summary(report, verdicts, baseline_path)

    if args.write_baseline:
        target = baseline_path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(write_baseline_bytes(report, args.profile), encoding="utf-8")
        print(f"  wrote baseline: {target}")

    # The report must stay readable by the schema gate that already exists.
    errors = validate_with_existing_checker(report)
    if errors:
        for error in errors:
            print(f"resource metrics: schema error: {error}", file=sys.stderr)
        return 1

    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))

    # `blocked` never fails the job by itself: it means the number could not be
    # produced honestly (no engine, no baseline yet), and pretending that is a
    # regression would train people to ignore the gate. Real regressions are
    # the only `fail`, and a baseline that never gets committed shows up as a
    # permanently blocked report in the artifact.
    return 0 if report["profiles"][0]["status"] != FAIL else 1


def validate_with_existing_checker(report):
    """Run `scripts/resource-efficiency-check.py`'s validator over the report.

    The harness must not emit a report its own gate would reject; keeping the
    two in step here means a schema change cannot pass one and fail the other.
    """
    import importlib.util

    path = REPO_ROOT / "scripts" / "resource-efficiency-check.py"
    spec = importlib.util.spec_from_file_location("resource_efficiency_check", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.validate(report)


# --------------------------------------------------------------------------
# Self-test
# --------------------------------------------------------------------------


def run_self_test():
    """Policy tests: every threshold must fail on a regression, pass on health.

    These are the tests that make the gate meaningful without a runner: they
    exercise the reduction and the verdicts over synthetic samples.
    """
    failures = []

    def check(condition, message):
        if not condition:
            failures.append(message)

    # -- reduction ------------------------------------------------------
    def samples(cpu_series, rss_series, step=0.25, start=100.0):
        return [
            Sample(start + index * step, cpu, rss, 12, "idle")
            for index, (cpu, rss) in enumerate(zip(cpu_series, rss_series))
        ]

    idle = samples([0.0, 0.0, 0.0, 0.0, 0.0], [50.0] * 5)
    check(cpu_percent(idle) == 0.0, "an idle process must read 0 % CPU")

    busy = samples([0.0, 0.25, 0.5, 0.75, 1.0], [50.0] * 5)
    check(
        abs(cpu_percent(busy) - 100.0) < 0.01,
        f"one full core must read 100 % CPU, got {cpu_percent(busy)}",
    )

    check(math.isnan(cpu_percent(idle[:1])), "one sample cannot produce a CPU rate")

    flat = samples([0.0] * 5, [50.0] * 5)
    check(abs(rss_slope_mb_per_min(flat)) < 1e-9, "a flat heap must have slope 0")

    # +6 MiB per minute: 0.25 s samples, 0.1 MiB each -> 24 MiB/min.
    growing = samples([0.0] * 6, [50.0 + 0.1 * index for index in range(6)])
    slope = rss_slope_mb_per_min(growing)
    check(
        abs(slope - 24.0) < 1e-6,
        f"a +0.4 MiB/s leak must read 24 MiB/min, got {slope}",
    )

    baseline = {
        "status": "measured",
        "metrics": {
            "idle_cpu_percent": 0.2,
            "rss_slope_mb_per_min": 0.1,
            "memory_growth_mb": 1.0,
            "startup_ms": 100.0,
            "p99_frame_time_ms": 5.0,
        },
    }
    healthy = dict(baseline["metrics"])
    check(
        overall_status(gate(healthy, baseline)) == PASS,
        "a measurement at the baseline must pass",
    )

    # -- each policy has to fail ----------------------------------------
    regressed = dict(healthy, idle_cpu_percent=3.0)
    verdicts = gate(regressed, baseline)
    check(
        overall_status(verdicts) == FAIL,
        "an idle CPU above baseline + 2 % must fail",
    )
    check(
        any(v["metric"] == "idle_cpu_percent" and v["status"] == FAIL for v in verdicts),
        "the failing metric must be named",
    )
    failing = next(v for v in verdicts if v["status"] == FAIL)
    check(
        "delta" in failing and "baseline" in failing and "measured" in failing,
        "a failure must name metric, baseline and delta",
    )

    check(
        overall_status(gate(dict(healthy, rss_slope_mb_per_min=5.0), baseline)) == FAIL,
        "a growing RSS slope above baseline + 4 MiB/min must fail",
    )
    check(
        overall_status(gate(dict(healthy, memory_growth_mb=65.0), baseline)) == FAIL,
        "memory growth above the 64 MiB budget must fail",
    )
    check(
        overall_status(gate(dict(healthy, startup_ms=601.0), baseline)) == FAIL,
        "a startup regression beyond 3x the baseline must fail",
    )
    check(
        overall_status(gate(dict(healthy, startup_ms=600.0), baseline)) == PASS,
        "startup noise within the 600 ms allowance must not fail a 100 ms baseline",
    )
    partial = gate({"startup_ms": 100.0}, baseline)
    check(
        next(v for v in partial if v["metric"] == "p99_frame_time_ms")["status"] == NA,
        "an unmeasured metric is n/a, not passed",
    )
    # The absolute target caps the relative allowance: a baseline that is
    # already near 2 % must not buy a larger budget.
    check(
        _allowed("idle_cpu_percent", 2.4, BUDGETS["idle_cpu_percent"]) == 2.0,
        "the absolute idle-CPU target must cap the baseline allowance",
    )

    no_baseline = gate(healthy, None)
    check(
        overall_status(no_baseline) == PASS,
        "a seed baseline must not fail a run",
    )
    check(
        all(v["status"] == BLOCKED for v in no_baseline),
        "without a baseline every metric must be recorded as blocked",
    )

    check(
        overall_status([{"metric": "x", "status": FAIL}]) == FAIL,
        "one failing verdict fails the run",
    )

    # -- status semantics ------------------------------------------------
    # A run that compared nothing must not report a pass.
    seed_status, seed_reason = run_status(no_baseline, "gstreamer", None, "ci-linux")
    check(
        seed_status == BLOCKED and "baseline" in (seed_reason or ""),
        f"a seed run must be blocked with a reason, got {seed_status}/{seed_reason}",
    )
    engine_status, engine_reason = run_status(
        no_baseline, "unavailable", "GStreamer initialization failed.", "ci-linux"
    )
    check(
        engine_status == BLOCKED and "GStreamer" in (engine_reason or ""),
        f"an unavailable engine must be blocked with its reason, got {engine_status}",
    )
    fail_status, fail_reason = run_status(verdicts, "gstreamer", None, "ci-linux")
    check(
        fail_status == FAIL and "idle_cpu_percent" in (fail_reason or ""),
        f"a failing metric must fail the run and be named, got {fail_status}/{fail_reason}",
    )
    pass_status, pass_reason = run_status(gate(healthy, baseline), "gstreamer", None, "ci-linux")
    check(
        pass_status == PASS and pass_reason is None,
        f"a healthy run must pass without a reason, got {pass_status}/{pass_reason}",
    )

    # A baseline is the reference everyone is compared against, so it must
    # describe the optimised binary users run.
    check(
        baseline_refusal("debug", False) is not None,
        "writing a baseline from a debug binary must be refused",
    )
    check(
        baseline_refusal("debug", True) is None,
        "--allow-debug-baseline must permit a throwaway local baseline",
    )
    check(
        baseline_refusal("release", False) is None,
        "a release baseline must be written without extra flags",
    )

    # -- the numbers that must never look healthy -----------------------
    check(
        gate({"idle_cpu_percent": float("nan")}, baseline)[0]["status"] == NA,
        "NaN is not a measurement",
    )
    check(
        _is_missing(None) and _is_missing(float("nan")) and not _is_missing(0.0),
        "absent or NaN values must not be serialised as measurements",
    )
    check(load_baseline(Path(tempfile.gettempdir()) / "no-such-baseline.json") is None,
          "a missing baseline is a seed, not a failure")
    check(
        load_baseline_store({"status": "seed"}) is None,
        "a seed baseline must not gate",
    )
    check(
        not _is_number(True),
        "a boolean is not a measurement",
    )

    # -- report shape ---------------------------------------------------
    # Everything `measure()` emits must satisfy the schema gate, including the
    # honest blocked/n/a records: a report its own gate rejects would fail the
    # job for the wrong reason.
    measured_profile = {
        "name": "self-test",
        "platform": "linux",
        "runner": "local",
        "binary_profile": "debug",
        "engine": "gstreamer",
        "engine_reason": None,
        "idle_seconds": 60.0,
        "settle_seconds": 2.0,
        "samples": 232,
        "preview_config": "scripts/ci-render-smoke/card.toml",
        "preview_frames": 30,
        "frame_time_reference": None,
        "capture_ab": {"status": NA, "reason": "not requested (--ab)"},
        "gpu": {"status": NA, "reason": "no GPU counter on this runner"},
        "power": {"status": NA, "reason": "no battery on this runner", "samples": []},
        "verdicts": [
            {
                "metric": "idle_cpu_percent",
                "status": PASS,
                "measured": 0.4,
                "baseline": 0.3,
                "delta": 0.1,
                "allowed": 2.0,
                "budget": {"absolute": 2.0},
                "reason": None,
            }
        ],
        "status": PASS,
        "threads_max": 14,
        "one_percent_low_fps": 55.0,
        "peak_rss_mb": 88.0,
        "p95_frame_time_ms": 4.0,
        "mean_frame_time_ms": 3.0,
        "max_frame_time_ms": 9.0,
        "engine_init_ms": 40.0,
        "frame_time_regression_percent": 0.0,
        "memory_growth_mb": 0.5,
        "cpu_delta_percent": 0.4,
        "idle_cpu_percent": 0.4,
        "p99_frame_time_ms": 5.0,
        "rss_slope_mb_per_min": 0.1,
        "startup_ms": 120.0,
        "frames_measured": 30,
        "not_measured": [],
    }
    report = {"schema_version": SCHEMA_VERSION, "profiles": [measured_profile]}
    check(
        validate_with_existing_checker(report) == [],
        f"a measured report must satisfy the schema gate: "
        f"{validate_with_existing_checker(report)}",
    )
    seed_profile = dict(
        measured_profile,
        status=BLOCKED,
        reason="no measured baseline for profile 'self-test'",
        verdicts=[
            {"metric": "startup_ms", "status": BLOCKED, "reason": "no baseline"}
        ],
    )
    check(
        validate_with_existing_checker(
            {"schema_version": SCHEMA_VERSION, "profiles": [seed_profile]}
        )
        == [],
        "a blocked seed report must satisfy the schema gate",
    )

    check(
        write_baseline_bytes(
            {
                "generated_at": "now",
                "profiles": [
                    {
                        "platform": "linux",
                        "runner": "local",
                        "binary_profile": "debug",
                        **healthy,
                    }
                ],
            },
            "self-test",
        ).find('"status": "measured"') > 0,
        "a written baseline must be marked as measured",
    )

    for failure in failures:
        print(f"self-test FAILED: {failure}", file=sys.stderr)
    if not failures:
        print("resource-metrics self-test: OK")
    return not failures


def load_baseline_store(data):
    """Apply the same seed rule as `load_baseline` to an in-memory baseline."""
    if data.get("status") != "measured":
        return None
    return data


if __name__ == "__main__":
    sys.exit(main())
