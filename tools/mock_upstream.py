#!/usr/bin/env python3
"""Tiny mock upstream used to verify YoyoRelay end to end."""
import json
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

BAD_KEY = "bad-key"
GOOD_KEY = "good-key"
ANTHROPIC_KEY = "good-anthropic-key"


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        sys.stderr.write("mock %s - %s\n" % (self.address_string(), fmt % args))

    def _auth(self):
        auth = self.headers.get("Authorization", "")
        if auth == f"Bearer {BAD_KEY}":
            return "bad"
        if auth == f"Bearer {GOOD_KEY}":
            return "good"
        if self.headers.get("x-api-key") == ANTHROPIC_KEY:
            return "anthropic"
        return "none"

    def _send(self, status, body, ctype="application/json", extra=None):
        data = body.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path.endswith("/models"):
            who = self._auth()
            if who == "bad":
                self._send(401, json.dumps({"error": {"message": "invalid api key"}}))
                return
            if who not in ("good", "anthropic"):
                self._send(401, json.dumps({"error": {"message": "missing api key"}}))
                return
            models = ["gpt-4o-upstream"] if who == "good" else ["claude-upstream"]
            self._send(200, json.dumps({"object": "list", "data": [{"id": m, "object": "model"} for m in models]}))
            return
        self._send(404, json.dumps({"error": {"message": "not found"}}))

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length) if length else b"{}"
        try:
            body = json.loads(raw.decode("utf-8") or "{}")
        except Exception:
            body = {}
        who = self._auth()
        stream = bool(body.get("stream"))
        if self.path.endswith("/chat/completions"):
            if who != "good":
                self._send(401 if who != "bad" else 401, json.dumps({"error": {"message": "invalid api key"}}))
                return
            if stream:
                self._sse_openai(body)
            else:
                self._send(200, json.dumps({
                    "id": "chatcmpl-mock",
                    "object": "chat.completion",
                    "created": int(time.time()),
                    "model": body.get("model", "mock"),
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "Hello from mock"},
                        "finish_reason": "stop",
                    }],
                    "usage": {"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18},
                }))
            return
        if self.path.endswith("/messages"):
            if who != "anthropic":
                self._send(401, json.dumps({"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}}))
                return
            if stream:
                self._sse_anthropic(body)
            else:
                self._send(200, json.dumps({
                    "id": "msg_mock",
                    "type": "message",
                    "role": "assistant",
                    "model": body.get("model", "mock"),
                    "content": [{"type": "text", "text": "Hello from anthropic mock"}],
                    "stop_reason": "end_turn",
                    "stop_sequence": None,
                    "usage": {"input_tokens": 9, "output_tokens": 4},
                }))
            return
        self._send(404, json.dumps({"error": {"message": "not found"}}))

    def _sse_openai(self, body):
        frames = []
        base = {"id": "chatcmpl-mock", "object": "chat.completion.chunk", "created": int(time.time()), "model": body.get("model", "mock")}
        def chunk(delta, finish=None, usage=None):
            obj = dict(base)
            obj["choices"] = [{"index": 0, "delta": delta, "finish_reason": finish}]
            if usage is not None:
                obj["usage"] = usage
            frames.append("data: " + json.dumps(obj, ensure_ascii=False) + "\n\n")
        chunk({"role": "assistant", "content": ""})
        chunk({"content": "Hello "})
        chunk({"content": "from mock stream"})
        chunk({}, "stop")
        chunk({}, None, {"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18})
        frames.append("data: [DONE]\n\n")
        self._send(200, "".join(frames), "text/event-stream")

    def _sse_anthropic(self, body):
        frames = []
        def ev(name, obj):
            frames.append(f"event: {name}\ndata: " + json.dumps(obj, ensure_ascii=False) + "\n\n")
        ev("message_start", {"type": "message_start", "message": {
            "id": "msg_mock", "type": "message", "role": "assistant",
            "model": body.get("model", "mock"), "content": [],
            "stop_reason": None, "stop_sequence": None,
            "usage": {"input_tokens": 9, "output_tokens": 0}}})
        ev("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}})
        ev("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hello "}})
        ev("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "from anthropic stream"}})
        ev("content_block_stop", {"type": "content_block_stop", "index": 0})
        ev("message_delta", {"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": None}, "usage": {"output_tokens": 5}})
        ev("message_stop", {"type": "message_stop"})
        self._send(200, "".join(frames), "text/event-stream")


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18080
    print(f"mock upstream on 127.0.0.1:{port}", flush=True)
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
