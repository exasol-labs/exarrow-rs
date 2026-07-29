"""Unit tests for scripts/strip_test_coverage.py.

Run with: python3 -m unittest discover -s scripts -p 'test_*.py'
"""

import json
import unittest

from strip_test_coverage import (
    CoverageFloorError,
    ProductionSourceMissingError,
    cfg_selects_test_only,
    check_floors,
    find_test_module_ranges,
    mask_non_code,
    strip_lcov,
    test_only_module_paths,
)


def blanked(prefix, literal, suffix):
    """The expected masking of `prefix + literal + suffix`."""
    return prefix + " " * len(literal) + suffix


class MaskNonCodeTest(unittest.TestCase):
    def assert_masks(self, prefix, literal, suffix):
        self.assertEqual(
            mask_non_code(prefix + literal + suffix), blanked(prefix, literal, suffix)
        )

    def test_line_comment_content_is_blanked(self):
        self.assert_masks("let a = 1; ", "// } } }", "\nlet b = 2;")

    def test_block_comment_content_is_blanked(self):
        self.assert_masks("a ", "/* } */", " b")

    def test_nested_block_comment_ends_at_outermost_terminator(self):
        self.assert_masks("a ", "/* /* } */ } */", " b")

    def test_block_comment_preserves_newlines(self):
        masked = mask_non_code("a /* }\n} */ b")
        self.assertEqual(masked, "a     \n     b")

    def test_string_literal_braces_are_blanked(self):
        self.assert_masks("let s = ", '"}{"', ";")

    def test_escaped_quote_does_not_terminate_string(self):
        self.assert_masks("let s = ", '"a\\"}"', "; x")

    def test_raw_string_with_hashes_blanks_inner_quotes_and_braces(self):
        self.assert_masks("let s = ", 'r#"a"}"#', "; x")

    def test_byte_raw_string_is_recognised(self):
        self.assert_masks("let s = ", 'br#"}"#', "; x")

    def test_identifier_ending_in_r_before_string_is_not_a_raw_string(self):
        self.assert_masks("for_r(", '"}"', "); }")

    def test_char_literal_brace_is_blanked(self):
        self.assert_masks("if c == ", "'}'", " { }")

    def test_escaped_quote_char_literal_does_not_open_a_string(self):
        self.assert_masks("if c == ", "'\\''", " { }")

    def test_escaped_unicode_char_literal_is_blanked(self):
        self.assert_masks("if c == ", "'\\u{7d}'", " { }")

    def test_escaped_hex_char_literal_is_blanked(self):
        self.assert_masks("if c == ", "'\\x7d'", " { }")

    def test_lifetime_is_not_treated_as_a_char_literal(self):
        self.assert_masks("fn f<'a>(x: &'a str) { let s = ", '"}"', "; }")

    def test_masking_preserves_length_and_line_structure(self):
        source = 'a\n// c\n"s"\n/* x\ny */\nb\n'
        masked = mask_non_code(source)
        self.assertEqual(len(masked), len(source))
        self.assertEqual(masked.count("\n"), source.count("\n"))


class CfgSelectsTestOnlyTest(unittest.TestCase):
    def test_bare_test_predicate_selects_test_only(self):
        self.assertTrue(cfg_selects_test_only("test"))

    def test_negated_test_predicate_is_production(self):
        self.assertFalse(cfg_selects_test_only("not(test)"))

    def test_all_with_test_member_selects_test_only(self):
        self.assertTrue(cfg_selects_test_only("all(test, unix)"))

    def test_any_with_non_test_member_is_not_test_only(self):
        self.assertFalse(cfg_selects_test_only("any(test, unix)"))

    def test_feature_predicate_is_not_test_only(self):
        self.assertFalse(cfg_selects_test_only("feature =      "))


class FindTestModuleRangesTest(unittest.TestCase):
    def test_finds_simple_test_module_range(self):
        source = "\n".join(
            [
                "pub fn a() {}",  # 1
                "",  # 2
                "#[cfg(test)]",  # 3
                "mod tests {",  # 4
                "    fn t() {}",  # 5
                "}",  # 6
                "pub fn b() {}",  # 7
            ]
        )
        self.assertEqual(find_test_module_ranges(source), [(3, 6)])

    def test_cfg_test_on_a_non_module_item_is_ignored(self):
        source = "#[cfg(test)]\nfn helper() {\n}\n"
        self.assertEqual(find_test_module_ranges(source), [])

    def test_cfg_test_module_declaration_without_body_is_ignored(self):
        source = "#[cfg(test)]\nmod tests;\nfn a() {}\n"
        self.assertEqual(find_test_module_ranges(source), [])

    def test_negated_cfg_test_module_is_not_stripped(self):
        source = "#[cfg(not(test))]\nmod real {\n}\n"
        self.assertEqual(find_test_module_ranges(source), [])

    def test_attributes_between_cfg_test_and_mod_are_skipped(self):
        source = "#[cfg(test)]\n#[allow(clippy::all)]\nmod tests {\n}\n"
        self.assertEqual(find_test_module_ranges(source), [(1, 4)])

    def test_public_test_module_is_detected(self):
        source = "#[cfg(test)]\npub(crate) mod tests {\n}\n"
        self.assertEqual(find_test_module_ranges(source), [(1, 3)])

    def test_brace_inside_string_does_not_close_the_module(self):
        source = "\n".join(
            [
                "#[cfg(test)]",  # 1
                "mod tests {",  # 2
                '    let s = "}";',  # 3
                "}",  # 4
                "pub fn a() {}",  # 5
            ]
        )
        self.assertEqual(find_test_module_ranges(source), [(1, 4)])

    def test_nested_module_is_covered_by_the_outer_range(self):
        source = "\n".join(
            [
                "#[cfg(test)]",  # 1
                "mod tests {",  # 2
                "    mod inner {",  # 3
                "        fn t() {}",  # 4
                "    }",  # 5
                "}",  # 6
            ]
        )
        self.assertEqual(find_test_module_ranges(source), [(1, 6)])

    def test_multiple_test_modules_are_all_reported(self):
        source = "\n".join(
            [
                "#[cfg(test)]",  # 1
                "mod a {",  # 2
                "}",  # 3
                "pub fn p() {}",  # 4
                "#[cfg(test)]",  # 5
                "mod b {",  # 6
                "}",  # 7
            ]
        )
        self.assertEqual(find_test_module_ranges(source), [(1, 3), (5, 7)])

    def test_unterminated_test_module_extends_to_end_of_file(self):
        source = "#[cfg(test)]\nmod tests {\n    fn t() {}\n"
        self.assertEqual(find_test_module_ranges(source), [(1, 3)])

    def test_cfg_test_module_on_one_line_is_detected(self):
        source = "pub fn a() {}\n#[cfg(test)] mod t { fn x() {} }\n"
        self.assertEqual(find_test_module_ranges(source), [(2, 2)])


class TestOnlyModulePathsTest(unittest.TestCase):
    def test_declaration_in_mod_rs_resolves_to_a_sibling_file(self):
        self.assertEqual(
            test_only_module_paths(
                "src/transport/mod.rs", "#[cfg(test)]\npub(crate) mod test_support;\n"
            ),
            {"src/transport/test_support.rs", "src/transport/test_support/mod.rs"},
        )

    def test_declaration_in_lib_rs_resolves_to_a_sibling_file(self):
        self.assertEqual(
            test_only_module_paths("src/lib.rs", "#[cfg(test)]\nmod fixtures;\n"),
            {"src/fixtures.rs", "src/fixtures/mod.rs"},
        )

    def test_declaration_in_a_named_file_resolves_into_its_own_directory(self):
        self.assertEqual(
            test_only_module_paths("src/query.rs", "#[cfg(test)]\nmod fixtures;\n"),
            {"src/query/fixtures.rs", "src/query/fixtures/mod.rs"},
        )

    def test_production_module_declaration_is_not_reported(self):
        self.assertEqual(
            test_only_module_paths("src/lib.rs", "pub mod query;\n"),
            set(),
        )

    def test_feature_gated_module_declaration_is_not_reported(self):
        self.assertEqual(
            test_only_module_paths(
                "src/lib.rs", '#[cfg(feature = "websocket")]\npub mod websocket;\n'
            ),
            set(),
        )

    def test_inline_test_module_is_not_reported_as_an_out_of_line_file(self):
        self.assertEqual(
            test_only_module_paths("src/lib.rs", "#[cfg(test)]\nmod tests {\n}\n"),
            set(),
        )


PROD_AND_TEST_SOURCE = "\n".join(
    [
        "pub fn a() -> u8 {",  # 1
        "    1",  # 2
        "}",  # 3
        "#[cfg(test)]",  # 4
        "mod tests {",  # 5
        "    #[test]",  # 6
        "    fn t() {",  # 7
        "        assert_eq!(super::a(), 1);",  # 8
        "    }",  # 9
        "}",  # 10
    ]
)


def lcov(*lines):
    return "\n".join(lines) + "\n"


class StripLcovTest(unittest.TestCase):
    def setUp(self):
        self.sources = {"src/a.rs": PROD_AND_TEST_SOURCE}

    def strip(self, text):
        return strip_lcov(text, self.sources.__getitem__)

    def test_removes_da_lines_inside_the_test_module(self):
        text = lcov(
            "SF:src/a.rs",
            "DA:1,5",
            "DA:2,5",
            "DA:7,5",
            "DA:8,5",
            "LF:4",
            "LH:4",
            "end_of_record",
        )
        stripped, _ = self.strip(text)
        self.assertNotIn("DA:7,5", stripped)
        self.assertNotIn("DA:8,5", stripped)
        self.assertIn("DA:1,5", stripped)
        self.assertIn("DA:2,5", stripped)

    def test_recomputes_lf_and_lh_from_the_kept_lines(self):
        text = lcov(
            "SF:src/a.rs",
            "DA:1,5",
            "DA:2,0",
            "DA:7,5",
            "LF:3",
            "LH:2",
            "end_of_record",
        )
        stripped, _ = self.strip(text)
        self.assertIn("LF:2", stripped)
        self.assertIn("LH:1", stripped)

    def test_recomputes_branch_totals_after_removing_test_branches(self):
        text = lcov(
            "SF:src/a.rs",
            "BRDA:1,0,0,3",
            "BRDA:1,0,1,-",
            "BRDA:8,0,0,3",
            "BRF:3",
            "BRH:2",
            "DA:1,5",
            "LF:1",
            "LH:1",
            "end_of_record",
        )
        stripped, _ = self.strip(text)
        self.assertNotIn("BRDA:8,0,0,3", stripped)
        self.assertIn("BRF:2", stripped)
        self.assertIn("BRH:1", stripped)

    def test_recomputes_function_totals_after_removing_test_functions(self):
        text = lcov(
            "SF:src/a.rs",
            "FN:1,_ZN1a",
            "FN:7,_ZN1t",
            "FNDA:5,_ZN1a",
            "FNDA:1,_ZN1t",
            "FNF:2",
            "FNH:2",
            "DA:1,5",
            "LF:1",
            "LH:1",
            "end_of_record",
        )
        stripped, _ = self.strip(text)
        self.assertNotIn("FN:7,_ZN1t", stripped)
        self.assertNotIn("FNDA:1,_ZN1t", stripped)
        self.assertIn("FN:1,_ZN1a", stripped)
        self.assertIn("FNF:1", stripped)
        self.assertIn("FNH:1", stripped)

    def test_file_whose_lines_are_all_test_lines_reports_zero_found(self):
        text = lcov("SF:src/a.rs", "DA:7,5", "DA:8,5", "LF:2", "LH:2", "end_of_record")
        stripped, summary = self.strip(text)
        self.assertIn("LF:0", stripped)
        self.assertIn("LH:0", stripped)
        self.assertIsNone(summary["files"][0]["percentage"])

    def test_preserves_unrelated_records_and_the_test_name_line(self):
        text = lcov("TN:", "SF:src/a.rs", "DA:1,5", "LF:1", "LH:1", "end_of_record")
        stripped, _ = self.strip(text)
        self.assertTrue(stripped.startswith("TN:\n"))
        self.assertTrue(stripped.endswith("end_of_record\n"))

    def test_missing_production_source_is_reported_not_silently_kept(self):
        text = lcov("SF:src/gone.rs", "DA:1,5", "LF:1", "LH:1", "end_of_record")
        with self.assertRaises(ProductionSourceMissingError):
            self.strip(text)

    def test_summary_totals_cover_every_file(self):
        self.sources["src/b.rs"] = "pub fn b() {}\n"
        text = lcov(
            "SF:src/a.rs",
            "DA:1,5",
            "DA:2,0",
            "DA:7,5",
            "LF:3",
            "LH:2",
            "end_of_record",
            "SF:src/b.rs",
            "DA:1,1",
            "LF:1",
            "LH:1",
            "end_of_record",
        )
        _, summary = self.strip(text)
        self.assertEqual(summary["lines_found"], 3)
        self.assertEqual(summary["lines_hit"], 2)
        self.assertAlmostEqual(summary["percentage"], 66.6667, places=3)

    def test_summary_files_are_sorted_ascending_by_percentage(self):
        self.sources["src/b.rs"] = "pub fn b() {}\n"
        text = lcov(
            "SF:src/a.rs",
            "DA:1,5",
            "LF:1",
            "LH:1",
            "end_of_record",
            "SF:src/b.rs",
            "DA:1,0",
            "LF:1",
            "LH:0",
            "end_of_record",
        )
        _, summary = self.strip(text)
        self.assertEqual(
            [f["file"] for f in summary["files"]], ["src/b.rs", "src/a.rs"]
        )

    def test_out_of_line_test_module_is_dropped_from_report_and_summary(self):
        self.sources["src/mod.rs"] = "#[cfg(test)]\nmod support;\npub fn p() {}\n"
        self.sources["src/support.rs"] = "pub fn mock() {}\n"
        text = lcov(
            "SF:src/mod.rs",
            "DA:3,1",
            "LF:1",
            "LH:1",
            "end_of_record",
            "SF:src/support.rs",
            "DA:1,0",
            "LF:1",
            "LH:0",
            "end_of_record",
        )
        stripped, summary = self.strip(text)
        self.assertNotIn("src/support.rs", stripped)
        self.assertEqual([f["file"] for f in summary["files"]], ["src/mod.rs"])
        self.assertEqual(summary["percentage"], 100.0)

    def test_summary_is_json_serialisable(self):
        text = lcov("SF:src/a.rs", "DA:1,5", "LF:1", "LH:1", "end_of_record")
        _, summary = self.strip(text)
        json.loads(json.dumps(summary))


def summary_of(percentage, files):
    return {
        "lines_found": 100,
        "lines_hit": int(percentage),
        "percentage": percentage,
        "files": [
            {"file": name, "lines_found": lf, "lines_hit": lh, "percentage": pct}
            for name, lf, lh, pct in files
        ],
    }


class CheckFloorsTest(unittest.TestCase):
    def test_passes_when_total_and_every_file_meet_the_floors(self):
        summary = summary_of(85.0, [("src/a.rs", 10, 9, 90.0)])
        check_floors(summary, min_total=80.0, min_per_file=50.0, exempt_files=())

    def test_fails_when_the_total_is_below_the_floor(self):
        summary = summary_of(79.9, [("src/a.rs", 10, 9, 90.0)])
        with self.assertRaises(CoverageFloorError) as ctx:
            check_floors(summary, min_total=80.0, min_per_file=50.0, exempt_files=())
        self.assertIn("79.9", str(ctx.exception))

    def test_fails_when_a_single_file_is_below_the_per_file_floor(self):
        summary = summary_of(85.0, [("src/low.rs", 10, 4, 40.0)])
        with self.assertRaises(CoverageFloorError) as ctx:
            check_floors(summary, min_total=80.0, min_per_file=50.0, exempt_files=())
        self.assertIn("src/low.rs", str(ctx.exception))

    def test_an_exempt_file_below_the_per_file_floor_does_not_fail_the_gate(self):
        summary = summary_of(85.0, [("src/low.rs", 10, 4, 40.0)])
        check_floors(
            summary, min_total=80.0, min_per_file=50.0, exempt_files=("src/low.rs",)
        )

    def test_a_file_with_no_production_lines_does_not_fail_the_gate(self):
        summary = summary_of(85.0, [("src/empty.rs", 0, 0, None)])
        check_floors(summary, min_total=80.0, min_per_file=50.0, exempt_files=())

    def test_reports_every_offending_file_not_only_the_first(self):
        summary = summary_of(
            85.0, [("src/x.rs", 10, 1, 10.0), ("src/y.rs", 10, 2, 20.0)]
        )
        with self.assertRaises(CoverageFloorError) as ctx:
            check_floors(summary, min_total=80.0, min_per_file=50.0, exempt_files=())
        self.assertIn("src/x.rs", str(ctx.exception))
        self.assertIn("src/y.rs", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
