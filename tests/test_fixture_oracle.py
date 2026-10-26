import hashlib
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from integration import expected_range


class PositionDistinctFixture(unittest.TestCase):
    def test_shifted_and_multipart_aligned_ranges_cannot_share_expected_bytes(self):
        positions = [0, 256, 4096, 65536, 1024 * 1024, 5 * 1024 * 1024]
        digests = {hashlib.sha256(expected_range(offset, 4096)).digest() for offset in positions}
        self.assertEqual(len(digests), len(positions))


if __name__ == "__main__":
    unittest.main()
