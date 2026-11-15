import io
import pathlib
import subprocess
import sys
import tarfile
import tempfile
import unittest


class ExtractionTest(unittest.TestCase):
    def extract(self, entries):
        with tempfile.TemporaryDirectory(prefix="umanager-mirror-archive-") as temporary:
            root = pathlib.Path(temporary)
            archive = root / "artifact.tar"
            with tarfile.open(archive, "w") as output:
                for name, kind in entries:
                    member = tarfile.TarInfo(name)
                    if kind == "link":
                        member.type = tarfile.SYMTYPE
                        member.linkname = "../outside"
                        output.addfile(member)
                    else:
                        member.size = 4
                        output.addfile(member, io.BytesIO(b"data"))
            result = subprocess.run([sys.executable, str(pathlib.Path(__file__).resolve().parents[1] / "extract-pages-artifact.py"), str(archive), str(root / "site")], capture_output=True)
            if result.returncode == 0:
                self.assertEqual((root / "site/feed.json").read_bytes(), b"data")
            self.assertFalse((root / "outside").exists())
            return result.returncode

    def test_regular_file(self):
        self.assertEqual(self.extract([("./feed.json", "file")]), 0)

    def test_traversal(self):
        self.assertNotEqual(self.extract([("../outside", "file")]), 0)

    def test_symlink(self):
        self.assertNotEqual(self.extract([("feed.json", "link")]), 0)

    def test_case_collision(self):
        self.assertNotEqual(self.extract([("feed.json", "file"), ("FEED.JSON", "file")]), 0)


if __name__ == "__main__":
    unittest.main()
