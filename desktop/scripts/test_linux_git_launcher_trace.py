#!/usr/bin/env python3

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from linux_git_launcher_trace import (
    VerificationError,
    cargo_test_executable,
    verify_trace,
)

REPO_ROOT = Path(__file__).resolve().parents[2]
PINNED_CONTAINER_IMAGE = (
    "ubuntu:24.04@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90"
)


class TraceVerifierTests(unittest.TestCase):
    def write_trace(self, root: Path, pid: int, body: str) -> None:
        (root / f"trace.{pid}").write_text(body.strip() + "\n")

    def good_traces(self, root: Path) -> None:
        self.write_trace(
            root,
            400,
            """
            clone(child_stack=NULL, flags=CLONE_VM|CLONE_VFORK|SIGCHLD) = 401
            clone3({flags=CLONE_VM|CLONE_VFORK, exit_signal=SIGCHLD}, 88) = 402
            clone(child_stack=0x1, flags=CLONE_VM|CLONE_THREAD|CLONE_SIGHAND) = 403
            """,
        )
        self.write_trace(
            root,
            401,
            """
            setpgid(0, 0) = 0
            chdir("/proc/self/fd/6") = 0
            execve("/usr/bin/git", ["/usr/bin/git", "--version"], 0x1) = 0
            """,
        )
        self.write_trace(
            root,
            402,
            """
            setpgid(0, 0) = 0
            chdir("/proc/self/fd/7") = 0
            execve("/usr/bin/git", ["/usr/bin/git", "hash-object", "--no-filters", "--", "marker"], 0x2) = 0
            """,
        )
        self.write_trace(root, 403, "+++ exited with 0 +++")

    def test_accepts_vfork_backends_per_expected_git_child(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.good_traces(root)
            verdict = verify_trace(root / "trace")

        self.assertEqual(verdict["verdict"], "pass")
        self.assertEqual(
            [child["backend"] for child in verdict["git_children"]],
            ["clone-vfork", "clone3-vfork"],
        )
        self.assertEqual(
            [child["descriptor_fd"] for child in verdict["git_children"]],
            [6, 7],
        )

    def test_rejects_fork_fallback_for_git_child(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.good_traces(root)
            parent = root / "trace.400"
            parent.write_text(parent.read_text().replace(
                "clone(child_stack=NULL, flags=CLONE_VM|CLONE_VFORK|SIGCHLD) = 401",
                "fork() = 401",
            ))
            with self.assertRaisesRegex(VerificationError, r"forbidden fork\(\) fallback"):
                verify_trace(root / "trace")

    def test_rejects_forked_intermediate_before_vfork_git_child(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.good_traces(root)
            parent = root / "trace.400"
            parent.write_text(parent.read_text().replace(
                "clone(child_stack=NULL, flags=CLONE_VM|CLONE_VFORK|SIGCHLD) = 401",
                "fork() = 410",
            ))
            self.write_trace(
                root,
                410,
                "clone(child_stack=NULL, flags=CLONE_VM|CLONE_VFORK|SIGCHLD) = 401",
            )
            with self.assertRaisesRegex(
                VerificationError, "unexpected non-thread process creation"
            ):
                verify_trace(root / "trace")

    def test_rejects_clone_without_vfork_for_git_child(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.good_traces(root)
            parent = root / "trace.400"
            parent.write_text(parent.read_text().replace(
                "CLONE_VM|CLONE_VFORK|SIGCHLD) = 401",
                "CLONE_VM|SIGCHLD) = 401",
            ))
            with self.assertRaisesRegex(VerificationError, "fork-style clone flags"):
                verify_trace(root / "trace")

    def test_rejects_clone_without_shared_vm_for_git_child(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.good_traces(root)
            parent = root / "trace.400"
            parent.write_text(parent.read_text().replace(
                "CLONE_VM|CLONE_VFORK|SIGCHLD) = 401",
                "CLONE_VFORK|SIGCHLD) = 401",
            ))
            with self.assertRaisesRegex(VerificationError, "CLONE_VM.*CLONE_VFORK"):
                verify_trace(root / "trace")

    def test_rejects_missing_descriptor_chdir(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.good_traces(root)
            child = root / "trace.401"
            child.write_text(child.read_text().replace(
                'chdir("/proc/self/fd/6") = 0\n', ""
            ))
            with self.assertRaisesRegex(VerificationError, "descriptor chdir"):
                verify_trace(root / "trace")

    def test_rejects_unexpected_git_invocation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.good_traces(root)
            child = root / "trace.402"
            child.write_text(child.read_text().replace("hash-object", "status"))
            with self.assertRaisesRegex(VerificationError, "unexpected Git child invocation set"):
                verify_trace(root / "trace")


class CargoExecutableTests(unittest.TestCase):
    def test_selects_the_only_buzz_lib_test_executable(self) -> None:
        messages = [
            json.dumps(
                {
                    "reason": "compiler-artifact",
                    "target": {
                        "name": "dependency",
                        "kind": ["lib"],
                        "test": True,
                        "src_path": "/dependency/src/lib.rs",
                    },
                    "profile": {"test": False},
                    "executable": None,
                }
            ),
            json.dumps(
                {
                    "reason": "compiler-artifact",
                    "target": {
                        "name": "buzz_lib",
                        "kind": ["staticlib", "cdylib", "rlib"],
                        "test": True,
                        "src_path": "/workspace/desktop/src-tauri/src/lib.rs",
                    },
                    "profile": {"test": True},
                    "executable": "/target/release/deps/buzz_lib-abc",
                }
            ),
        ]
        self.assertEqual(
            cargo_test_executable(messages),
            Path("/target/release/deps/buzz_lib-abc"),
        )

    def test_rejects_ambiguous_test_executables(self) -> None:
        messages = [
            json.dumps(
                {
                    "reason": "compiler-artifact",
                    "target": {
                        "name": "buzz_lib",
                        "kind": ["staticlib", "cdylib", "rlib"],
                        "test": True,
                        "src_path": "/workspace/desktop/src-tauri/src/lib.rs",
                    },
                    "profile": {"test": True},
                    "executable": f"/target/release/deps/buzz_lib-{suffix}",
                }
            )
            for suffix in ("abc", "def")
        ]
        with self.assertRaisesRegex(VerificationError, "expected one buzz_lib"):
            cargo_test_executable(messages)


class WorkflowWiringTests(unittest.TestCase):
    def test_ci_release_and_canary_use_the_pinned_runtime_gate(self) -> None:
        workflows = (
            REPO_ROOT / ".github/workflows/ci.yml",
            REPO_ROOT / ".github/workflows/release.yml",
            REPO_ROOT / ".github/workflows/linux-canary.yml",
        )
        for workflow in workflows:
            with self.subTest(workflow=workflow.name):
                source = workflow.read_text()
                self.assertIn(f"container: {PINNED_CONTAINER_IMAGE}", source)
                self.assertIn(
                    f"SCHOOLX_LAUNCHER_CONTAINER_IMAGE: {PINNED_CONTAINER_IMAGE}",
                    source,
                )
                self.assertIn(
                    "desktop/scripts/verify-linux-git-launcher-runtime.sh",
                    source,
                )
                self.assertIn("linux-git-launcher-evidence", source)
                self.assertRegex(source, r"(?m)^\s+strace \\\s*$")
                self.assertRegex(source, r"(?m)^\s+util-linux \\\s*$")

        runtime_gate = (
            REPO_ROOT / "desktop/scripts/verify-linux-git-launcher-runtime.sh"
        ).read_text()
        self.assertNotRegex(
            runtime_gate,
            r"(?m)^\s*strace\s+[^\n]*\s-v(?:\s|\\|$)",
            "raw trace evidence must not expand inherited environment values",
        )
        self.assertIn('HOME="$evidence_dir/home"', runtime_gate)
        self.assertIn(
            'HERMIT_STATE_DIR="$parent_hermit_state_dir"', runtime_gate
        )
        self.assertNotIn('env HOME="$HOME"', runtime_gate)
        self.assertNotRegex(
            runtime_gate,
            r"rustc\s+-vV\s*\|\s*grep\s+[^\n]*-q",
            "pipefail must not turn grep -q's early exit into a rustc failure",
        )
        self.assertNotRegex(
            runtime_gate,
            r"file\s+-Lb\s+[^\n|]+\|\s*grep\s+[^\n]*-q",
            "pipefail must not turn grep -q's early exit into a file failure",
        )


if __name__ == "__main__":
    unittest.main()
