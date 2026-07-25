#!/usr/bin/env python3

import argparse
import csv
import json
import os
import platform
import shlex
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


BENCHMARK_ROOT = Path(__file__).resolve().parent
REPOSITORY_ROOT = BENCHMARK_ROOT.parent
RESULTS_ROOT = BENCHMARK_ROOT / "results"


def configure_console():
    for stream in (sys.stdout, sys.stderr):
        reconfigure = getattr(stream, "reconfigure", None)
        if reconfigure:
            reconfigure(encoding="utf-8", errors="replace")


def capture(*command):
    try:
        result = subprocess.run(
            command,
            cwd=REPOSITORY_ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            encoding="utf-8",
            errors="replace",
            check=False,
        )
    except OSError as error:
        return f"unavailable: {error}"
    return result.stdout.strip()


def cpu_name():
    if sys.platform == "win32":
        try:
            import winreg

            key_path = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0"
            with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, key_path) as key:
                return winreg.QueryValueEx(key, "ProcessorNameString")[0].strip()
        except OSError:
            pass
    elif sys.platform.startswith("linux"):
        try:
            for line in Path("/proc/cpuinfo").read_text(
                encoding="utf-8", errors="replace"
            ).splitlines():
                key, separator, value = line.partition(":")
                if separator and key.strip() in {"model name", "Hardware"}:
                    return value.strip()
        except OSError:
            pass
    elif sys.platform == "darwin":
        value = capture("sysctl", "-n", "machdep.cpu.brand_string")
        if value and not value.startswith("unavailable:"):
            return value
        value = capture("sysctl", "-n", "hw.model")
        if value and not value.startswith("unavailable:"):
            return value
    return platform.processor() or platform.machine() or "unknown"


def power_context():
    if sys.platform == "win32":
        return capture("powercfg", "/GETACTIVESCHEME")
    if sys.platform.startswith("linux"):
        governor = Path(
            "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"
        )
        try:
            return f"CPU scaling governor: {governor.read_text().strip()}"
        except OSError:
            return "unavailable"
    if sys.platform == "darwin":
        return capture("pmset", "-g", "batt")
    return "unavailable"


def command_text(command):
    return shlex.join(command)


def metadata(command, benchmark_filter):
    return {
        "timestamp_utc": datetime.now(timezone.utc).isoformat(),
        "command": command_text(command),
        "filter": benchmark_filter,
        "repository": str(REPOSITORY_ROOT),
        "git_commit": capture("git", "rev-parse", "HEAD"),
        "git_status": capture("git", "status", "--short"),
        "rustc": capture("rustc", "--version", "--verbose"),
        "cargo": capture("cargo", "--version"),
        "cpu": {
            "name": cpu_name(),
            "logical_processors": os.cpu_count(),
        },
        "os": {
            "system": platform.system(),
            "release": platform.release(),
            "version": platform.version(),
            "machine": platform.machine(),
        },
        "power_context": power_context(),
        "criterion": "0.8.2",
        "criterion_defaults": {
            "warm_up_seconds": 3,
            "measurement_seconds": 5,
            "samples": 100,
            "confidence_level": 0.95,
            "bootstrap_resamples": 100_000,
        },
        "startup_override": {
            "measurement_seconds": 10,
            "samples": 30,
            "sampling_mode": "flat",
        },
    }


def format_duration(nanoseconds):
    if nanoseconds >= 1e9:
        return f"{nanoseconds / 1e9:.3f} s"
    if nanoseconds >= 1e6:
        return f"{nanoseconds / 1e6:.3f} ms"
    if nanoseconds >= 1e3:
        return f"{nanoseconds / 1e3:.3f} us"
    return f"{nanoseconds:.3f} ns"


def result_rows(criterion_directory):
    rows = []
    for estimate_path in criterion_directory.rglob("estimates.json"):
        if estimate_path.parent.name != "new":
            continue
        benchmark_directory = estimate_path.parent.parent
        try:
            benchmark = benchmark_directory.relative_to(
                criterion_directory
            ).as_posix()
        except ValueError as error:
            raise ValueError(
                f"estimate is outside the Criterion directory: {estimate_path}"
            ) from error
        with estimate_path.open(encoding="utf-8") as estimate_file:
            estimate = json.load(estimate_file)
        rows.append(
            {
                "benchmark": benchmark,
                "mean_ns": float(estimate["mean"]["point_estimate"]),
                "mean_lower_95_ns": float(
                    estimate["mean"]["confidence_interval"]["lower_bound"]
                ),
                "mean_upper_95_ns": float(
                    estimate["mean"]["confidence_interval"]["upper_bound"]
                ),
                "median_ns": float(estimate["median"]["point_estimate"]),
            }
        )
    rows.sort(key=lambda row: row["benchmark"])
    if not rows:
        raise ValueError(
            f"no completed Criterion estimates found in {criterion_directory}"
        )
    return rows


def describe_cpu(run_metadata):
    cpu = run_metadata["cpu"]
    description = cpu["name"]
    physical = cpu.get("physical_cores")
    logical = cpu.get("logical_processors")
    if physical and logical:
        return f"{description}, {physical} cores / {logical} threads"
    if logical:
        return f"{description}, {logical} logical processors"
    return description


def describe_os(run_metadata):
    operating_system = run_metadata["os"]
    if "caption" in operating_system:
        return (
            f"{operating_system['caption']} {operating_system['version']}, "
            f"build {operating_system['build']}"
        )
    values = [
        operating_system.get("system"),
        operating_system.get("release"),
        operating_system.get("version"),
        operating_system.get("machine"),
    ]
    return " ".join(value for value in values if value)


def summarize(result_directory):
    result_directory = result_directory.resolve(strict=True)
    criterion_directory = (result_directory / "criterion").resolve(strict=True)
    metadata_path = result_directory / "metadata.json"
    with metadata_path.open(encoding="utf-8-sig") as metadata_file:
        run_metadata = json.load(metadata_file)
    rows = result_rows(criterion_directory)

    with (result_directory / "summary.csv").open(
        "w", encoding="utf-8", newline=""
    ) as summary_file:
        writer = csv.DictWriter(summary_file, fieldnames=rows[0].keys())
        writer.writeheader()
        writer.writerows(rows)

    rustc = run_metadata["rustc"].splitlines()[0]
    lines = [
        "# Oxyst benchmark results",
        "",
        f"- Run (UTC): {run_metadata['timestamp_utc']}",
        f"- Commit: `{run_metadata['git_commit']}`",
        f"- CPU: {describe_cpu(run_metadata)}",
        f"- OS: {describe_os(run_metadata)}",
        f"- Toolchain: {rustc}",
        "- Method: Criterion.rs 0.8.2 wall-clock benchmark, release profile, "
        "95% confidence intervals",
        "",
        "| Benchmark | Mean | 95% confidence interval | Median |",
        "|---|---:|---:|---:|",
    ]
    for row in rows:
        lines.append(
            f"| `{row['benchmark']}` | {format_duration(row['mean_ns'])} | "
            f"{format_duration(row['mean_lower_95_ns'])} - "
            f"{format_duration(row['mean_upper_95_ns'])} | "
            f"{format_duration(row['median_ns'])} |"
        )
    lines.extend(
        [
            "",
            "See `metadata.json` for the exact environment and command, "
            "`console.log` for Criterion's full analysis and throughput output, "
            "`summary.csv` for machine-readable estimates, and `criterion/` for "
            "raw samples and baselines.",
        ]
    )
    (result_directory / "summary.md").write_text(
        "\n".join(lines) + "\n", encoding="utf-8"
    )
    return len(rows)


def run_benchmark(benchmark_filter):
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    result_directory = RESULTS_ROOT / timestamp
    criterion_directory = result_directory / "criterion"
    log_path = result_directory / "console.log"
    result_directory.mkdir(parents=True)

    command = [
        "cargo",
        "bench",
        "-p",
        "oxyst-benchmarks",
        "--bench",
        "operations",
        "--",
        "--noplot",
        "--color",
        "never",
    ]
    if benchmark_filter:
        command.append(benchmark_filter)

    (result_directory / "metadata.json").write_text(
        json.dumps(metadata(command, benchmark_filter), indent=2) + "\n",
        encoding="utf-8",
    )
    environment = os.environ.copy()
    environment["CRITERION_HOME"] = str(criterion_directory)

    try:
        process = subprocess.Popen(
            command,
            cwd=REPOSITORY_ROOT,
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            encoding="utf-8",
            errors="replace",
        )
    except OSError as error:
        raise RuntimeError(f"could not start Cargo: {error}") from error

    with log_path.open("w", encoding="utf-8") as log_file:
        for line in process.stdout:
            sys.stdout.write(line)
            sys.stdout.flush()
            log_file.write(line)
            log_file.flush()
    exit_code = process.wait()
    if exit_code:
        (result_directory / "FAILED.md").write_text(
            f"# Benchmark failed\n\nCargo exited with code {exit_code}. "
            "See `console.log`.\n",
            encoding="utf-8",
        )
        raise RuntimeError(
            f"benchmark failed with exit code {exit_code}; see {log_path}"
        )

    count = summarize(result_directory)
    print(f"\nResults ({count} benchmarks): {result_directory}")


def parse_arguments():
    parser = argparse.ArgumentParser(
        description="Run and summarize the Oxyst Criterion benchmarks."
    )
    actions = parser.add_mutually_exclusive_group()
    actions.add_argument(
        "--filter",
        default="",
        help="Criterion benchmark-name regular expression.",
    )
    actions.add_argument(
        "--summarize",
        type=Path,
        metavar="RESULT_DIRECTORY",
        help="Regenerate summaries for an existing result directory.",
    )
    return parser.parse_args()


def main():
    configure_console()
    arguments = parse_arguments()
    try:
        if arguments.summarize:
            count = summarize(arguments.summarize)
            print(
                f"Summarized {count} benchmarks in "
                f"{arguments.summarize.resolve()}"
            )
        else:
            run_benchmark(arguments.filter)
    except (KeyError, OSError, RuntimeError, ValueError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
