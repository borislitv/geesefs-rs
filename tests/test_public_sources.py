import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from audit_public_sources import PUBLIC_REGISTRY, violations


class PublicSourceAudit(unittest.TestCase):
    def setUp(self):
        self.root = Path("/disposable/project")
        self.manifest = {"lints": {"rust": {"unsafe_code": "forbid"}}}
        self.metadata = {
            "workspace_members": ["root"],
            "packages": [
                {"id": "root", "name": "root", "manifest_path": str(self.root / "Cargo.toml"), "license": "Apache-2.0"},
                {"id": "dep", "name": "dep", "source": PUBLIC_REGISTRY, "license_file": "LICENSE"},
            ],
        }

    def test_public_registry_and_license_file_are_valid(self):
        self.assertEqual(violations(self.metadata, self.manifest, self.root), [])

    def test_private_git_registry_and_external_paths_are_rejected(self):
        for origin in [None, "git+https://private.invalid/repository", "registry+https://private.invalid/index"]:
            metadata = copy.deepcopy(self.metadata)
            metadata["packages"][1]["source"] = origin
            self.assertIn("non-public dependency source: dep", violations(metadata, self.manifest, self.root))

    def test_unsafe_policy_cannot_be_relaxed(self):
        for policy in ["allow", "warn", "deny", None]:
            manifest = {"lints": {"rust": {"unsafe_code": policy}}}
            self.assertTrue(violations(self.metadata, manifest, self.root))

    def test_workspace_members_cannot_hide_an_external_dependency(self):
        self.metadata["workspace_members"].append("dep")
        self.metadata["packages"][1]["manifest_path"] = "/external/Cargo.toml"
        self.assertIn("unexpected workspace source: dep", violations(self.metadata, self.manifest, self.root))


if __name__ == "__main__":
    unittest.main()
