import os
import subprocess
import unittest
from unittest import mock

import ones_list_releases


class OnesListReleasesTest(unittest.TestCase):
    def test_run_ones_includes_stdout_payload_on_failure(self):
        result = subprocess.CompletedProcess(
            [],
            1,
            '{"code":401,"errcode":"AuthFailure.InvalidToken"}',
            '',
        )
        with mock.patch('ones_list_releases.subprocess.run', return_value=result):
            with self.assertRaises(RuntimeError) as ctx:
                ones_list_releases._run_ones(['graphql', '--compact', '{ tasks { uuid } }'])

        message = str(ctx.exception)
        self.assertIn('exit=1', message)
        self.assertIn('stdout=', message)
        self.assertIn('AuthFailure.InvalidToken', message)

    def test_run_ones_redacts_sensitive_env_values(self):
        result = subprocess.CompletedProcess(
            [],
            1,
            '{"token":"raw-token"}',
            'password=raw-password token=json-token',
        )
        env = {
            'ONES_PASSWORD': 'raw-password',
            'ONES_TOKEN': 'raw-token',
            'ONES_TOKEN_JSON': '{"uuid":"u","token":"json-token"}',
        }
        with mock.patch.dict(os.environ, env, clear=True):
            with mock.patch('ones_list_releases.subprocess.run', return_value=result):
                with self.assertRaises(RuntimeError) as ctx:
                    ones_list_releases._run_ones(['graphql', '--compact', '{ tasks { uuid } }'])

        message = str(ctx.exception)
        self.assertIn('<redacted>', message)
        self.assertNotIn('raw-password', message)
        self.assertNotIn('raw-token', message)
        self.assertNotIn('json-token', message)


if __name__ == '__main__':
    unittest.main()
