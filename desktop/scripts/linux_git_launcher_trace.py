#!/usr/bin/env python3
"""Verify the Linux descriptor-bound Git launcher's release strace evidence."""

from __future__ import annotations

import argparse
import ast
import json
import re
import sys
from pathlib import Path
from typing import Any, Iterable


EXPECTED_GIT_INVOCATIONS = (
    ("/usr/bin/git", "--version"),
    ("/usr/bin/git", "hash-object", "--no-filters", "--", "marker"),
)


class VerificationError(ValueError):
    """Raised when build or trace evidence does not prove the contract."""


def cargo_test_executable(lines: Iterable[str]) -> Path:
    """Return the sole release lib-test executable from Cargo JSON output."""

    candidates: set[Path] = set()
    for line in lines:
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if message.get("reason") != "compiler-artifact":
            continue
        target = message.get("target", {})
        profile = message.get("profile", {})
        executable = message.get("executable")
        if (
            target.get("name") == "buzz_lib"
            and target.get("test") is True
            and str(target.get("src_path", "")).endswith("/src/lib.rs")
            and profile.get("test") is True
            and executable
        ):
            candidates.add(Path(executable))

    if len(candidates) != 1:
        rendered = ", ".join(sorted(str(path) for path in candidates)) or "none"
        raise VerificationError(
            f"expected one buzz_lib Cargo test executable, found {len(candidates)} ({rendered})"
        )
    return candidates.pop()


_TRACE_CALL_RE = re.compile(r"^(fork|vfork|clone|clone3)\((.*)\)\s+=\s+(\d+)$")
_FLAGS_RE = re.compile(r"\bflags=([A-Z0-9_|]+)")
_EXEC_RE = re.compile(
    r'^execve\(("(?:\\.|[^"\\])*"), \[(.*?)\], .*\)\s+=\s+0$'
)
_C_STRING_RE = re.compile(r'"(?:\\.|[^"\\])*"')
_SETPGID_RE = re.compile(r"^setpgid\(0,\s*0\)\s+=\s+0$")
_CHDIR_RE = re.compile(r'^chdir\("/proc/self/fd/([0-9]+)"\)\s+=\s+0$')


def _decode_c_string(token: str) -> str:
    try:
        value = ast.literal_eval(token)
    except (SyntaxError, ValueError) as error:
        raise VerificationError(f"could not decode strace string {token!r}: {error}") from error
    if not isinstance(value, str):
        raise VerificationError(f"strace token was not a string: {token!r}")
    return value


def _successful_exec(line: str) -> tuple[str, tuple[str, ...]] | None:
    match = _EXEC_RE.match(line)
    if not match:
        return None
    executable = _decode_c_string(match.group(1))
    argv = tuple(_decode_c_string(token) for token in _C_STRING_RE.findall(match.group(2)))
    if not argv:
        raise VerificationError(f"successful execve had an empty argv: {line}")
    return executable, argv


def _trace_files(prefix: Path) -> dict[int, tuple[Path, list[str]]]:
    files: dict[int, tuple[Path, list[str]]] = {}
    for path in sorted(prefix.parent.glob(f"{prefix.name}.*")):
        suffix = path.name.removeprefix(f"{prefix.name}.")
        if not suffix.isdigit() or not path.is_file():
            continue
        pid = int(suffix)
        lines = [line.strip() for line in path.read_text(errors="replace").splitlines()]
        files[pid] = (path, lines)
    if not files:
        raise VerificationError(f"no per-process strace files matched {prefix}.*")
    return files


def _process_creations(
    files: dict[int, tuple[Path, list[str]]],
) -> Iterable[tuple[int, int, str, tuple[str, ...]]]:
    for parent_pid, (_, lines) in files.items():
        for line in lines:
            match = _TRACE_CALL_RE.match(line)
            if not match:
                continue
            call, arguments, child = match.group(1), match.group(2), match.group(3)
            flags_match = _FLAGS_RE.search(arguments)
            flags = tuple(flags_match.group(1).split("|")) if flags_match else ()
            yield parent_pid, int(child), call, flags


def _creator_for_pid(
    child_pid: int, files: dict[int, tuple[Path, list[str]]]
) -> tuple[int, str, tuple[str, ...]]:
    creators: list[tuple[int, str, tuple[str, ...]]] = []
    for parent_pid, created_pid, call, flags in _process_creations(files):
        if created_pid == child_pid:
            creators.append((parent_pid, call, flags))

    if len(creators) != 1:
        raise VerificationError(
            f"Git child {child_pid} had {len(creators)} successful process creators; expected one"
        )
    return creators[0]


def _verify_backend(child_pid: int, call: str, flags: tuple[str, ...]) -> str:
    if call == "fork":
        raise VerificationError(f"Git child {child_pid} used forbidden fork() fallback")
    if call == "vfork":
        return "vfork"
    if call not in {"clone", "clone3"}:
        raise VerificationError(f"Git child {child_pid} used unknown creator {call}")
    if (
        "CLONE_VFORK" not in flags
        or "CLONE_VM" not in flags
        or "CLONE_THREAD" in flags
    ):
        rendered = "|".join(flags) or "<missing>"
        raise VerificationError(
            f"Git child {child_pid} used fork-style {call} flags {rendered}; "
            "CLONE_VM|CLONE_VFORK is required"
        )
    return f"{call}-vfork"


def verify_trace(prefix: Path) -> dict[str, Any]:
    """Verify every expected Git exec and its direct process-creation backend."""

    files = _trace_files(prefix)
    children: list[dict[str, Any]] = []
    observed_invocations: list[tuple[str, ...]] = []

    for child_pid, (path, lines) in files.items():
        git_execs: list[tuple[int, tuple[str, ...]]] = []
        for index, line in enumerate(lines):
            execution = _successful_exec(line)
            if execution is None or execution[0] != "/usr/bin/git":
                continue
            executable, invocation = execution
            if not invocation or invocation[0] != executable:
                raise VerificationError(
                    f"Git child {child_pid} execve executable and argv[0] differed"
                )
            git_execs.append((index, invocation))
        if not git_execs:
            continue
        if len(git_execs) != 1:
            raise VerificationError(
                f"Git child trace {path.name} had {len(git_execs)} successful /usr/bin/git execs"
            )

        exec_index, invocation = git_execs[0]
        setpgid_indexes = [index for index, line in enumerate(lines) if _SETPGID_RE.match(line)]
        descriptor_cwds = [
            (index, int(match.group(1)))
            for index, line in enumerate(lines)
            if (match := _CHDIR_RE.match(line))
        ]
        if len(setpgid_indexes) != 1:
            raise VerificationError(
                f"Git child {child_pid} had {len(setpgid_indexes)} successful setpgid(0, 0) calls"
            )
        if len(descriptor_cwds) != 1:
            raise VerificationError(
                f"Git child {child_pid} had {len(descriptor_cwds)} successful descriptor chdir calls"
            )
        chdir_index, descriptor_fd = descriptor_cwds[0]
        if descriptor_fd < 3:
            raise VerificationError(
                f"Git child {child_pid} used standard descriptor {descriptor_fd} as cwd authority"
            )
        if not setpgid_indexes[0] < chdir_index < exec_index:
            raise VerificationError(
                f"Git child {child_pid} did not order setpgid -> descriptor chdir -> execve"
            )

        parent_pid, creator_call, flags = _creator_for_pid(child_pid, files)
        backend = _verify_backend(child_pid, creator_call, flags)
        observed_invocations.append(invocation)
        children.append(
            {
                "pid": child_pid,
                "parent_pid": parent_pid,
                "trace_file": path.name,
                "backend": backend,
                "creator": creator_call,
                "clone_flags": list(flags),
                "descriptor_fd": descriptor_fd,
                "argv": list(invocation),
            }
        )

    expected = sorted(EXPECTED_GIT_INVOCATIONS)
    observed = sorted(observed_invocations)
    if observed != expected:
        raise VerificationError(
            "unexpected Git child invocation set: "
            f"expected {expected!r}, observed {observed!r}"
        )

    git_child_pids = {int(child["pid"]) for child in children}
    for parent_pid, child_pid, call, flags in _process_creations(files):
        if call in {"clone", "clone3"} and "CLONE_THREAD" in flags:
            continue
        if child_pid not in git_child_pids:
            rendered = "|".join(flags) or "<none>"
            raise VerificationError(
                "unexpected non-thread process creation: "
                f"parent {parent_pid} used {call} ({rendered}) for child {child_pid}; "
                "only the two expected Git children are allowed"
            )

    order = {invocation: index for index, invocation in enumerate(EXPECTED_GIT_INVOCATIONS)}
    children.sort(key=lambda child: order[tuple(child["argv"])])
    return {
        "schema_version": 1,
        "verdict": "pass",
        "policy": {
            "git_executable": "/usr/bin/git",
            "required_cwd": "/proc/self/fd/<non-stdio-fd>",
            "required_process_group": "setpgid(0, 0)",
            "allowed_backends": [
                "vfork",
                "clone(CLONE_VM|CLONE_VFORK)",
                "clone3(CLONE_VM|CLONE_VFORK)",
            ],
            "forbidden_backend": "fork-style creation without CLONE_VM|CLONE_VFORK",
            "allowed_non_thread_children": "the two expected Git children only",
        },
        "git_children": children,
        "trace_files": [path.name for path, _ in files.values()],
    }


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    cargo = subparsers.add_parser("cargo-executable")
    cargo.add_argument("json_messages", type=Path)

    trace = subparsers.add_parser("verify")
    trace.add_argument("trace_prefix", type=Path)
    trace.add_argument("--output", type=Path)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _build_parser().parse_args(argv)
    try:
        if args.command == "cargo-executable":
            with args.json_messages.open() as messages:
                print(cargo_test_executable(messages))
            return 0

        verdict = verify_trace(args.trace_prefix)
        rendered = json.dumps(verdict, indent=2, sort_keys=True) + "\n"
        if args.output:
            args.output.write_text(rendered)
        else:
            print(rendered, end="")
        return 0
    except (OSError, VerificationError) as error:
        print(f"linux Git launcher evidence rejected: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
