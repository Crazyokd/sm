import os
import io
import json
import unittest
import urllib.error
from unittest import mock

import ones_api


class _FakeResponse:
    def __init__(self, status, payload):
        self.status = status
        self.payload = payload

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False

    def getcode(self):
        return self.status

    def read(self):
        return json.dumps(self.payload).encode('utf-8')


class OnesApiAuthTest(unittest.TestCase):
    def test_auth_from_env_supports_ones_token_json(self):
        with mock.patch.dict(
            os.environ,
            {'ONES_TOKEN_JSON': '{"uuid":"user-from-json","token":"token-from-json"}'},
            clear=True,
        ):
            self.assertEqual(
                ones_api._auth_from_env(),
                ('user-from-json', 'token-from-json'),
            )

    def test_auth_from_env_pair_has_priority_over_ones_token_json(self):
        with mock.patch.dict(
            os.environ,
            {
                'ONES_USER_ID': 'user-from-pair',
                'ONES_AUTH_TOKEN': 'token-from-pair',
                'ONES_TOKEN_JSON': '{"uuid":"user-from-json","token":"token-from-json"}',
            },
            clear=True,
        ):
            self.assertEqual(
                ones_api._auth_from_env(),
                ('user-from-pair', 'token-from-pair'),
            )

    def test_auth_from_env_ignores_invalid_ones_token_json(self):
        with mock.patch.dict(os.environ, {'ONES_TOKEN_JSON': 'not-json'}, clear=True):
            self.assertIsNone(ones_api._auth_from_env())

    def test_auth_from_env_ignores_non_object_ones_token_json(self):
        with mock.patch.dict(os.environ, {'ONES_TOKEN_JSON': '[]'}, clear=True):
            self.assertIsNone(ones_api._auth_from_env())

    def test_raw_request_refreshes_invalid_env_token_with_password_login(self):
        unauthorized = urllib.error.HTTPError(
            url='https://ones.example/graphql',
            code=401,
            msg='Unauthorized',
            hdrs=None,
            fp=io.BytesIO(b'{"code":401,"errcode":"AuthFailure.InvalidToken"}'),
        )
        login_response = _FakeResponse(
            200,
            {'user': {'uuid': 'new-user', 'token': 'new-token'}},
        )
        retry_response = _FakeResponse(200, {'ok': True})

        env = {
            'ONES_TOKEN_JSON': '{"uuid":"old-user","token":"old-token"}',
            'ONES_EMAIL': 'bot@example.com',
            'ONES_PASSWORD': 'secret-password',
        }
        with mock.patch.dict(os.environ, env, clear=True):
            with mock.patch('ones_api._write_token') as write_token:
                with mock.patch(
                    'ones_api.urllib.request.urlopen',
                    side_effect=[unauthorized, login_response, retry_response],
                ):
                    status, payload = ones_api.raw_request(
                        'GET',
                        '/team/UNrQ5Ny5/tasks',
                        auth=('old-user', 'old-token'),
                    )

        self.assertEqual(status, 200)
        self.assertEqual(payload, {'ok': True})
        write_token.assert_called_once_with('new-user', 'new-token')


if __name__ == '__main__':
    unittest.main()
