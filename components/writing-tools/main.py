"""Writing Tools: a sample Folio component.

Folio and a component exchange one JSON object per line over stdin/stdout.
See docs/components.md for the full protocol.
"""

import datetime
import json
import sys

GOALS = [None, 250, 500, 1000, 2000]
goal = None
next_id = 0
backlog = []  # messages that arrived while waiting for a reply


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def request(method, params=None):
    """Sends a request to Folio and waits for its reply."""
    global next_id
    next_id += 1
    send({"id": next_id, "method": method, "params": params or {}})
    for line in sys.stdin:
        message = json.loads(line)
        if message.get("id") == next_id and "method" not in message:
            if "error" in message:
                raise RuntimeError(message["error"])
            return message.get("result")
        backlog.append(message)
    sys.exit(0)


def notify(text):
    send({"method": "notify", "params": {"text": text}})


def show_goal(words):
    text = None if goal is None else f"{words} / {goal} words"
    send({"method": "status.set", "params": {"text": text}})

    if goal is None:
        lines = ["No goal yet.", {"text": "Set a goal →", "command": "goal"}]
    else:
        done = min(words / goal, 1.0)
        filled = round(done * 16)
        lines = [
            "█" * filled + "░" * (16 - filled) + f"  {round(done * 100)}%",
            f"{words} of {goal} words",
            "Goal reached!" if words >= goal else f"{goal - words} to go",
            {"text": "Next goal →", "command": "goal"},
        ]
    send({"method": "panel.set", "params": {"id": "goal", "lines": lines}})


def plural(n, word):
    return f"{n} {word}" if n == 1 else f"{n} {word}s"


def count_words(text):
    return len(text.split())


def handle(message):
    global goal
    method = message.get("method")
    params = message.get("params", {})

    if method == "initialize":
        show_goal(0)
    elif method == "command":
        command = params["command"]
        if command == "goal":
            goal = GOALS[(GOALS.index(goal) + 1) % len(GOALS)]
            show_goal(count_words(request("doc.text")["text"]))
        elif command == "date":
            request("doc.insert", {"text": datetime.date.today().strftime("%B %-d, %Y")})
        elif command == "stats":
            doc = request("doc.get")
            paragraphs = doc["paragraphs"]
            headings = sum(1 for p in paragraphs if p["style"] in ("title", "h1", "h2"))
            text = " ".join(r["text"] for p in paragraphs for r in p["runs"])
            words = count_words(text)
            minutes = max(1, round(words / 230))
            notify(
                f"{plural(len(paragraphs), 'paragraph')}, {plural(headings, 'heading')}, "
                f"{plural(words, 'word')}, about {minutes} min to read"
            )
    elif method == "event" and params.get("kind") == "changed":
        show_goal(params["words"])
    elif method == "shutdown":
        sys.exit(0)


for line in sys.stdin:
    handle(json.loads(line))
    while backlog:
        handle(backlog.pop(0))
