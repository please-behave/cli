"""Offline coverage for the repository's Gemini protocol adapter."""
from copy import deepcopy
import importlib.util
import json
import os
from pathlib import Path
import unittest
from unittest.mock import MagicMock, patch

spec = importlib.util.spec_from_file_location(
    "gemini", Path(__file__).resolve().parents[1] / ".please/adapters/gemini.py")
gemini = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gemini)


class GeminiAdapterTests(unittest.TestCase):
    def setUp(self):
        self.request = {"protocol_version": 1, "prompt": "Complete prepared evidence",
                        "reviews": [{"id": "guidance", "parameters": {"model": "fixture-model"}}]}

    def test_protocol_request_reaches_http_and_verdict_is_wrapped(self):
        verdict = {"reviews": [{"id": "guidance", "status": "PASS", "reason": "Clear guidance",
                                "findings": [{"path": "AGENTS.md", "line": 1, "message": "Actionable"}]}]}
        response = MagicMock()
        response.status = 200
        response.read.return_value = json.dumps({"candidates": [{"finishReason": "STOP",
            "content": {"parts": [{"text": json.dumps(verdict)}]}}]}).encode()
        opener = MagicMock()
        opener.open.return_value.__enter__.return_value = response
        with patch.dict(os.environ, GEMINI_API_KEY="fixture-key"), patch.object(gemini, "build_opener", return_value=opener):
            result = gemini.review(self.request)
        self.assertEqual(result, {"protocol_version": 1, **verdict})
        request = opener.open.call_args.args[0]
        self.assertIn("/models/fixture-model:generateContent", request.full_url)
        self.assertEqual(json.loads(request.data)["contents"][0]["parts"][0]["text"], self.request["prompt"])

    def test_invalid_protocol_and_missing_or_conflicting_models_never_call_provider(self):
        invalid = []
        for version in (True, 2):
            request = deepcopy(self.request)
            request["protocol_version"] = version
            invalid.append(request)
        request = deepcopy(self.request)
        request["reviews"][0]["parameters"] = {}
        invalid.append(request)
        request = deepcopy(self.request)
        request["reviews"].append({"id": "other", "parameters": {"model": "different-model"}})
        invalid.append(request)
        with patch.object(gemini, "run") as run:
            for request in invalid:
                with self.subTest(request=request), self.assertRaises(gemini.ProviderError):
                    gemini.review(request)
            run.assert_not_called()

    def test_provider_failure_returns_error_for_every_requested_review(self):
        self.request["reviews"].append({"id": "other", "parameters": {"model": "fixture-model"}})
        with patch.object(gemini, "run", side_effect=gemini.ProviderError("Gemini API returned HTTP 401.")):
            result = gemini.review(self.request)
        self.assertEqual(result["protocol_version"], 1)
        self.assertEqual([item["id"] for item in result["reviews"]], ["guidance", "other"])
        self.assertTrue(all(item["status"] == "ERROR" for item in result["reviews"]))

    def test_invalid_json_and_duplicate_keys_do_not_become_pass(self):
        for raw in ('not JSON', '{"reviews": [], "reviews": []}'):
            with self.subTest(raw=raw), patch.object(gemini, "run", return_value=raw):
                with self.assertRaises(ValueError):
                    gemini.review(self.request)


if __name__ == "__main__":
    unittest.main()
