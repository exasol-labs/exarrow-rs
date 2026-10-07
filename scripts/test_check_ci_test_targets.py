"""Unit tests for scripts/check_ci_test_targets.py.

Run with: python3 -m unittest discover -s scripts -p 'test_*.py'
"""

import contextlib
import io
import tempfile
import unittest
from pathlib import Path

from check_ci_test_targets import main



def workflow(*integration_steps: str, other_steps: str = "") -> str:
    """Build a workflow whose `integration-tests` job holds the given step lines."""
    steps = "\n".join(f"      {line}" for step in integration_steps for line in step.splitlines())
    return (
        "jobs:\n"
        "  lint:\n"
        "    steps:\n"
        f"{other_steps}"
        "  integration-tests:\n"
        "    steps:\n"
        f"{steps}\n"
        "  release:\n"
        "    steps:\n"
        "      - run: echo done\n"
    )


WORKFLOW_WITH_BOTH_TARGETS = workflow(
    "- run: cargo test --features ffi --test alpha_tests -- --test-threads=1",
    "- run: cargo test --test beta_tests",
)


class CheckCiTestTargetsTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        self.tests_dir = root / "tests"
        self.tests_dir.mkdir()
        self.workflow = root / "ci.yml"
        self.workflow.write_text(WORKFLOW_WITH_BOTH_TARGETS)
        (self.tests_dir / "alpha_tests.rs").write_text("#[test]\nfn a() {}\n")
        (self.tests_dir / "beta_tests.rs").write_text("#[test]\nfn b() {}\n")

    def run_check(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = main([str(self.tests_dir), str(self.workflow)])
        return code, out.getvalue()

    def test_every_target_present_exits_zero(self):
        """Scenario: CI rejects an integration test target that the integration job does not run"""
        code, output = self.run_check()
        self.assertEqual(code, 0)
        self.assertEqual(output, "")

    def test_missing_target_exits_one_and_names_the_target(self):
        """Scenario: CI rejects an integration test target that the integration job does not run"""
        (self.tests_dir / "gamma_tests.rs").write_text("")
        code, output = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("gamma_tests", output)
        self.assertNotIn("alpha_tests", output)

    def test_target_with_longer_name_does_not_count(self):
        """Scenario: CI rejects an integration test target that the integration job does not run"""
        self.workflow.write_text(
            workflow(
                "- run: cargo test --test alpha_tests_extra",
                "- run: cargo test --test beta_tests",
            )
        )
        code, output = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("alpha_tests", output)
        self.assertNotIn("beta_tests", output)

    def test_target_at_end_of_file_without_newline_counts(self):
        """Scenario: CI rejects an integration test target that the integration job does not run"""
        self.workflow.write_text(
            workflow(
                "- run: cargo test --test alpha_tests --x",
                "- run: cargo test --test beta_tests",
            ).rstrip("\n")
        )
        code, _ = self.run_check()
        self.assertEqual(code, 0)

    def assert_alpha_not_run(self, text):
        self.workflow.write_text(text)
        code, output = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("alpha_tests", output)
        self.assertNotIn("beta_tests", output)

    def test_commented_out_invocation_does_not_count(self):
        """Scenario: CI rejects an integration test target that the integration job does not run"""
        self.assert_alpha_not_run(
            workflow(
                "# - run: cargo test --test alpha_tests",
                "- run: |",
                "    # cargo test --test alpha_tests",
                "    cargo test --test beta_tests",
            )
        )

    def test_trailing_comment_does_not_count(self):
        """Scenario: CI rejects an integration test target that the integration job does not run"""
        self.assert_alpha_not_run(
            workflow(
                "- run: cargo test --test beta_tests # --test alpha_tests",
            )
        )

    def test_echoed_invocation_does_not_count(self):
        """Scenario: CI rejects an integration test target that the integration job does not run"""
        self.assert_alpha_not_run(
            workflow(
                "- run: echo cargo test --test alpha_tests",
                "- run: |",
                "    echo \"cargo test --test alpha_tests\"",
                "    cargo test --test beta_tests",
            )
        )

    def test_invocation_in_another_job_does_not_count(self):
        """Scenario: CI rejects an integration test target that the integration job does not run"""
        self.assert_alpha_not_run(
            workflow(
                "- run: cargo test --test beta_tests",
                other_steps="      - run: cargo test --test alpha_tests\n",
            )
        )

    def test_multiline_run_block_counts(self):
        """Scenario: CI rejects an integration test target that the integration job does not run"""
        self.workflow.write_text(
            workflow(
                "- name: Run both",
                "  run: |",
                "    cargo test --test alpha_tests",
                "    cargo test --test beta_tests -- --x",
            )
        )
        code, _ = self.run_check()
        self.assertEqual(code, 0)

    def test_missing_integration_job_reports_every_target(self):
        """Scenario: CI rejects an integration test target that the integration job does not run"""
        self.workflow.write_text("jobs:\n  lint:\n    steps:\n      - run: cargo test --test alpha_tests\n")
        code, output = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("alpha_tests", output)
        self.assertIn("beta_tests", output)

    def test_bare_ignore_exits_one_and_names_file_and_line(self):
        """Scenario: CI rejects an integration test target that the integration job does not run"""
        path = self.tests_dir / "alpha_tests.rs"
        path.write_text("#[test]\n    #[ignore]\nfn a() {}\n")
        code, output = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn(f"{path}:2", output)

    def test_bare_ignore_in_subdirectory_is_found(self):
        """Scenario: CI rejects an integration test target that the integration job does not run"""
        sub = self.tests_dir / "common"
        sub.mkdir()
        (sub / "mod.rs").write_text("#[ignore]\n")
        code, output = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn(f"{sub / 'mod.rs'}:1", output)

    def test_subdirectory_file_is_not_a_test_target(self):
        """Scenario: CI rejects an integration test target that the integration job does not run"""
        sub = self.tests_dir / "common"
        sub.mkdir()
        (sub / "mod.rs").write_text("")
        code, _ = self.run_check()
        self.assertEqual(code, 0)

    def test_ignore_with_reason_exits_zero(self):
        """Scenario: CI rejects an integration test target that the integration job does not run"""
        (self.tests_dir / "alpha_tests.rs").write_text(
            '#[test]\n#[ignore = "needs a long run"]\nfn a() {}\n'
        )
        code, _ = self.run_check()
        self.assertEqual(code, 0)


if __name__ == "__main__":
    unittest.main()
