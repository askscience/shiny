#!/usr/bin/env python3
"""Tests for scripts/plugin-catalog.py. Run: python3 scripts/test-plugin-catalog.py"""
import importlib.util
import pathlib
import unittest

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("plugin_catalog", HERE / "plugin-catalog.py")
catalog = importlib.util.module_from_spec(spec)
spec.loader.exec_module(catalog)


def entry(name, version, min_host=None, arch="amd64", sha="x"):
    made = {"name": name, "version": version, "arch": arch, "sha256": sha}
    if min_host:
        made["min_host"] = min_host
    return made


def index(*entries):
    return {"format": 2, "release": "0.3.0", "plugins": list(entries)}


def listed(merged):
    return [(e["name"], e["version"]) for e in merged["plugins"]]


class Pending(unittest.TestCase):
    plugins = {
        "image": {"version": "0.1.2", "runtime": "process"},
        "calc": {"version": "0.1.4", "runtime": "process"},
    }

    def test_everything_is_pending_before_the_first_publication(self):
        self.assertEqual(catalog.pending(index(), self.plugins, "amd64"), ["image", "calc"])

    def test_only_a_version_that_went_up_is_pending(self):
        published = index(entry("image", "0.1.2"), entry("calc", "0.1.3"))
        self.assertEqual(catalog.pending(published, self.plugins, "amd64"), ["calc"])

    def test_versions_compare_by_number_not_by_text(self):
        plugins = {"image": {"version": "0.1.10", "runtime": "process"}}
        self.assertEqual(catalog.pending(index(entry("image", "0.1.9")), plugins, "amd64"), ["image"])

    def test_an_older_version_in_the_repository_is_not_published(self):
        published = index(entry("image", "0.2.0"), entry("calc", "0.1.4"))
        self.assertEqual(catalog.pending(published, self.plugins, "amd64"), [])

    def test_another_architecture_does_not_count_as_published(self):
        published = index(entry("image", "0.1.2", arch="arm64"), entry("calc", "0.1.4"))
        self.assertEqual(catalog.pending(published, self.plugins, "amd64"), ["image"])

    def test_a_malformed_version_or_min_host_stops_the_publication(self):
        with self.assertRaises(ValueError):
            catalog.pending(index(), {"image": {"version": "next", "runtime": "process"}}, "amd64")
        with self.assertRaises(ValueError):
            catalog.pending(
                index(), {"image": {"version": "0.1.2", "min_host": "soon", "runtime": "process"}}, "amd64"
            )


class Merge(unittest.TestCase):
    def test_a_new_version_for_every_server_replaces_the_old_one(self):
        merged = catalog.merge(index(entry("image", "0.1.1"), entry("calc", "0.1.3")), index(entry("image", "0.1.2")))
        self.assertEqual(listed(merged), [("calc", "0.1.3"), ("image", "0.1.2")])

    def test_a_version_for_newer_servers_keeps_the_one_older_servers_run(self):
        merged = catalog.merge(index(entry("image", "0.1.2")), index(entry("image", "0.1.3", "0.3.1")))
        self.assertEqual(listed(merged), [("image", "0.1.2"), ("image", "0.1.3")])

    def test_a_version_no_server_would_pick_is_dropped(self):
        published = index(entry("image", "0.1.2"), entry("image", "0.1.3", "0.3.1"))
        # 0.1.4 also needs 0.3.1: every server that could run 0.1.3 takes 0.1.4.
        merged = catalog.merge(published, index(entry("image", "0.1.4", "0.3.1")))
        self.assertEqual(listed(merged), [("image", "0.1.2"), ("image", "0.1.4")])
        # A later version that runs everywhere supersedes all of them.
        merged = catalog.merge(merged, index(entry("image", "0.1.5")))
        self.assertEqual(listed(merged), [("image", "0.1.5")])

    def test_a_rebuilt_version_replaces_its_published_entry(self):
        merged = catalog.merge(index(entry("image", "0.1.2", sha="old")), index(entry("image", "0.1.2", sha="new")))
        self.assertEqual([e["sha256"] for e in merged["plugins"]], ["new"])

    def test_architectures_are_kept_apart(self):
        merged = catalog.merge(index(entry("image", "0.1.1", arch="arm64")), index(entry("image", "0.1.2")))
        self.assertEqual(
            [(e["arch"], e["version"]) for e in merged["plugins"]], [("amd64", "0.1.2"), ("arm64", "0.1.1")]
        )

    def test_the_first_publication_starts_from_nothing(self):
        merged = catalog.merge(catalog.load_index("/nonexistent/index.json"), index(entry("image", "0.1.2")))
        self.assertEqual(listed(merged), [("image", "0.1.2")])
        self.assertEqual(merged["format"], 2)


class Repository(unittest.TestCase):
    def test_the_repository_plugins_are_process_plugins_with_valid_versions(self):
        plugins = catalog.repo_plugins()
        self.assertIn("image", plugins)
        self.assertNotIn("hello", plugins)
        for library in ("terminal", "updates", "installer"):
            self.assertNotIn(library, plugins)
        # Nothing raises: every version and min_host in the tree is well formed.
        catalog.pending(index(), plugins, "amd64")


if __name__ == "__main__":
    unittest.main()
