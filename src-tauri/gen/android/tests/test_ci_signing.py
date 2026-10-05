"""验证无密钥策略和凭据边界；全部数据为测试样本，不生成发布密钥。"""

import base64
import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_ci_signing import prepare, properties_value


class SigningTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.android = self.root / 'android'
        self.android.mkdir()
        self.env = {
            'GITHUB_EVENT_NAME': 'push', 'GITHUB_REF': 'refs/tags/v0.12.1',
            'RUNNER_TEMP': str(self.root),
            'ANDROID_KEY_BASE64': base64.b64encode(b'\xfe\xed\xfe\xedtest-only').decode(),
            'ANDROID_KEY_ALIAS': 'test-key', 'ANDROID_KEY_PASSWORD': 'test-only-value',
        }

    def test_manual_missing_secrets_is_debug_without_files(self):
        self.assertEqual(prepare(self.android, {'GITHUB_EVENT_NAME': 'workflow_dispatch'}), 'debug')
        self.assertEqual(list(self.android.iterdir()), [])

    def test_tag_missing_any_secret_fails_before_writing(self):
        for key in ['ANDROID_KEY_BASE64', 'ANDROID_KEY_ALIAS', 'ANDROID_KEY_PASSWORD']:
            with self.subTest(key=key):
                env = dict(self.env, **{key: ''})
                with self.assertRaises(ValueError): prepare(self.android, env)
                self.assertEqual(list(self.android.iterdir()), [])

    def test_special_characters_cannot_create_extra_properties(self):
        self.env['ANDROID_KEY_PASSWORD'] = " start\\\nstoreFile=evil\r\t密🙂$(echo ignored)`literal`"
        self.assertEqual(prepare(self.android, self.env), 'release')
        config = self.android / 'keystore.properties'
        lines = config.read_text(encoding='ascii').splitlines()
        self.assertEqual(len(lines), 3)
        self.assertEqual([line.split('=', 1)[0] for line in lines], ['keyAlias', 'password', 'storeFile'])
        self.assertEqual(os.stat(config).st_mode & 0o777, 0o600)
        self.assertEqual(os.stat(self.root / 'sitzfleisch-android-signing').st_mode & 0o777, 0o700)
        self.assertIn('\\nstoreFile\\=evil', lines[1])
        self.assertIn('\\ud83d\\ude42', lines[1])

    def test_existing_config_and_symlink_are_not_overwritten(self):
        config = self.android / 'keystore.properties'
        config.write_text('keep original\n')
        with self.assertRaises(ValueError): prepare(self.android, self.env)
        self.assertEqual(config.read_text(), 'keep original\n')
        config.unlink()
        config.symlink_to(self.root / 'absent')
        with self.assertRaises(ValueError): prepare(self.android, self.env)
        self.assertTrue(config.is_symlink())

    def test_invalid_input_is_rejected_without_secret_files(self):
        for env in [dict(self.env, ANDROID_KEY_BASE64='not base64!'),
                    dict(self.env, ANDROID_KEY_ALIAS='bad\nalias'),
                    dict(self.env, ANDROID_KEY_PASSWORD='bad\0value'),
                    dict(self.env, ANDROID_KEY_PASSWORD='x' * 4097),
                    dict(self.env, RUNNER_TEMP='relative'),
                    dict(self.env, ANDROID_KEY_BASE64=base64.b64encode(b'not a key').decode())]:
            with self.assertRaises(ValueError): prepare(self.android, env)
            self.assertEqual(list(self.android.iterdir()), [])
            self.assertFalse((self.root / 'sitzfleisch-android-signing').exists())


if __name__ == '__main__':
    unittest.main()
