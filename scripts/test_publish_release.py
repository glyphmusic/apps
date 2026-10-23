import hashlib
import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('publisher', Path(__file__).with_name('publish-release.py'))
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)


class PublicationTests(unittest.TestCase):
    def test_complete_build_and_windows_binary_marker(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            names = publisher.expected_assets('server', '0.8.0')
            for name in names:
                (root / name).write_bytes(name.encode())
                digest = hashlib.sha256(name.encode()).hexdigest()
                (root / (name + '.sha256')).write_text(f'{digest} *{name}\n')
            text = publisher.manifest(root, names)
            self.assertEqual(len(text.splitlines()), 6)
            for name in names:
                self.assertIn(f'  {name}\n', text)
            changed = next(iter(names))
            (root / changed).write_bytes(b'different run')
            with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                publisher.manifest(root, names)

    def test_missing_extra_wrong_name_and_duplicate_entries(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            name = 'exact.zip'
            (root / name).write_bytes(b'content')
            digest = hashlib.sha256(b'content').hexdigest()
            sidecar = root / (name + '.sha256')
            with self.assertRaisesRegex(ValueError, 'inventory'):
                publisher.manifest(root, {name})
            for entry in [f'{digest}  other.zip\n', f'{digest}  {name}\n' * 2]:
                sidecar.write_text(entry)
                with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                    publisher.manifest(root, {name})
            sidecar.write_text(f'{digest}  {name}\n')
            (root / 'unexpected.zip').write_bytes(b'extra')
            with self.assertRaisesRegex(ValueError, 'inventory'):
                publisher.manifest(root, {name})

    def test_published_release_and_interrupted_upload_cannot_be_overwritten(self):
        with self.assertRaisesRegex(ValueError, 'already published'):
            publisher.require_unpublished({'draft': False, 'assets': []})
        with self.assertRaisesRegex(ValueError, 'already contains assets'):
            publisher.require_unpublished({'draft': True, 'assets': [{'name': 'old.zip'}]})
        publisher.require_unpublished({'draft': True, 'assets': []})

    def test_app_inventory_includes_every_installer_and_android(self):
        names = publisher.expected_assets('apps', '0.8.0')
        self.assertEqual(len(names), 13)
        self.assertIn('glyph-0.8.0-android.apk', names)
        self.assertIn('glyph-0.8.0-windows-x64-setup.exe', names)
        self.assertTrue(all('0.8.0' in name for name in names))


if __name__ == '__main__':
    unittest.main()
