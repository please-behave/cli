"""Count literal Markdown tokens separated by ASCII whitespace, independently of locale."""
from pathlib import Path
import re
import sys


def main(path, limit):
    if limit < 1:
        raise ValueError("Word limit must be positive")
    text = Path(path).read_text(encoding="utf-8")
    count = len(re.findall(r"[^ \t\n\r\v\f]+", text))
    print(f"{path}: {count} words (limit {limit})")
    return int(count > limit)


if __name__ == "__main__":
    try:
        if len(sys.argv) != 3:
            raise ValueError("Usage: check-word-count.py PATH LIMIT")
        sys.exit(main(sys.argv[1], int(sys.argv[2])))
    except (OSError, ValueError) as error:
        sys.exit(f"Word-count check failed: {error}")
