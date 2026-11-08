"""Send one complete Hannes prompt to Gemini; Hannes validates the extracted verdict."""
from email.utils import parsedate_to_datetime
import errno
import http.client
import json
import os
import socket
import ssl
import sys
import time
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import Request, build_opener, HTTPRedirectHandler

LIMIT = 65536
DELAYS = (30, 60, 120, 240)
RETRYABLE = (429, 500, 502, 503, 504)


class ProviderError(ValueError):
    """Safe diagnostic without request or response content."""


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        # Never forward the API key to a redirect target.
        return None


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("Duplicate envelope key")
        result[key] = value
    return result


def unwrap(raw):
    try:
        if len(raw) > LIMIT:
            raise ValueError()
        envelope = json.loads(raw, object_pairs_hook=unique_object)
        if not isinstance(envelope, dict) or "error" in envelope or envelope.get("promptFeedback", {}).get("blockReason"):
            raise ValueError()
        candidate = envelope["candidates"][0]
        if candidate["finishReason"] != "STOP":
            raise ValueError()
        parts = candidate["content"]["parts"]
        if not isinstance(parts, list):
            raise ValueError()
        text = []
        for part in parts:
            if not isinstance(part, dict) or part.get("thought"):
                raise ValueError()
            value = part.get("text")
            if not isinstance(value, str):
                raise ValueError()
            text.append(value)
        response = "".join(text)
        if not response.strip():
            raise ValueError()
        return response
    except (ValueError, KeyError, IndexError, TypeError, AttributeError):
        raise ProviderError("Gemini API returned an invalid or incomplete response.") from None


def retry_delay(headers, fallback):
    value = headers.get("Retry-After", "")
    try:
        seconds = int(value)
    except ValueError:
        try:
            seconds = parsedate_to_datetime(value).timestamp() - time.time()
        except (ValueError, TypeError, OverflowError):
            return fallback
    return min(240, max(fallback, seconds))


def transient(error):
    return not isinstance(error, ssl.SSLError) and (
        isinstance(error, (ConnectionError, http.client.RemoteDisconnected))
        or isinstance(error, socket.gaierror) and error.errno == socket.EAI_AGAIN
        or getattr(error, "errno", None) in (errno.ECONNRESET, errno.ECONNREFUSED, errno.ENETUNREACH, errno.EHOSTUNREACH)
    )


def inspect_error(error):
    # Parse the bounded error envelope if present. HTTP status alone controls retry;
    # API messages/status strings are untrusted and never become diagnostics.
    try:
        envelope = json.loads(error.read(LIMIT + 1))
        if isinstance(envelope, dict):
            envelope.get("error", {})
    except TimeoutError:
        raise ProviderError("Gemini API request timed out; no retry.") from None
    except (ValueError, OSError, http.client.HTTPException):
        pass
    finally:
        error.close()


def run(prompt):
    key, model = os.environ.get("GEMINI_API_KEY"), os.environ.get("HANNES_MODEL")
    if not key or not model:
        raise ProviderError("Gemini API requires GEMINI_API_KEY and HANNES_MODEL.")
    if isinstance(prompt, bytes):
        prompt = prompt.decode("utf-8")
    payload = json.dumps({"contents": [{"role": "user", "parts": [{"text": prompt}]}],
                          "generationConfig": {"responseMimeType": "application/json", "maxOutputTokens": 8192}}).encode()
    encoded_model = quote(model, safe="")
    request = Request(f"https://generativelanguage.googleapis.com/v1beta/models/{encoded_model}:generateContent",
                      data=payload, headers={"Content-Type": "application/json", "x-goog-api-key": key}, method="POST")
    opener = build_opener(NoRedirect())
    for attempt in range(5):
        delay = DELAYS[min(attempt, 3)]
        try:
            with opener.open(request, timeout=600) as response:
                if not 200 <= response.status < 300:
                    raise ProviderError("Gemini API returned an unsuccessful HTTP response.")
                return unwrap(response.read(LIMIT + 1))
        except HTTPError as error:
            status = error.code
            delay = retry_delay(error.headers, delay)
            inspect_error(error)
            if status not in RETRYABLE:
                raise ProviderError(f"Gemini API returned HTTP {status}.") from None
            diagnostic = f"Gemini API returned HTTP {status}"
        except (TimeoutError, URLError, OSError, http.client.HTTPException) as error:
            reason = error.reason if isinstance(error, URLError) else error
            if isinstance(reason, TimeoutError):
                raise ProviderError("Gemini API request timed out; no retry.") from None
            if not transient(reason):
                raise ProviderError("Gemini API connection failed; no retry.") from None
            diagnostic = "Gemini API connection failed"
        if attempt == 4:
            raise ProviderError(f"{diagnostic}; exhausted 5 attempts.")
        print(f"{diagnostic}; retrying (attempt {attempt + 2}/5) in {delay:g} seconds.", file=sys.stderr, flush=True)
        time.sleep(delay)


if __name__ == "__main__":
    try:
        sys.stdout.write(run(sys.stdin.buffer.read()))
    except ProviderError as error:
        sys.exit(str(error))
    except (OSError, ValueError, KeyError):
        sys.exit("Gemini API review failed; raw output suppressed.")
