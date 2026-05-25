import pathlib
import re
import sys


LINE_RE = re.compile(r"^( *)(\S+?)(?:@([^:]+):(\d+)\.\.(\d+))?(?:\s.*)?$")
KEYWORD_MAP = {
    "UseItem": "use",
    "ModItem": "mod",
    "Struct": "struct",
    "Union": "union",
    "Enum": "enum",
    "Int": "int",
    "Bool": "_Bool",
    "Char": "char",
    "Float": "float",
    "Double": "double",
    "Long": "long",
    "Short": "short",
    "Signed": "signed",
    "Unsigned": "unsigned",
    "Void": "void",
    "Const": "const",
    "Volatile": "volatile",
    "Static": "static",
    "Extern": "extern",
    "Typedef": "typedef",
    "Auto": "auto",
    "Register": "register",
    "Constexpr": "constexpr",
    "Inline": "inline",
    "PushOnly": "__ccc_pack_push_only",
    "Pop": "__ccc_pack_pop",
    "Reset": "__ccc_pack_reset",
}
IDENTIFIER_LABELS = {"Field", "FieldName", "Name", "Alias", "Segment", "LastParam", "Ident"}


def load_file(root: pathlib.Path, relative: str, cache: dict[str, str]) -> str:
    if relative not in cache:
        cache[relative] = (root / relative).read_text()
    return cache[relative]


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: validate_spans.py <test-dir> <dump-file>", file=sys.stderr)
        return 2

    root = pathlib.Path(sys.argv[1])
    dump = pathlib.Path(sys.argv[2]).read_text().splitlines()
    file_cache: dict[str, str] = {}
    errors: list[str] = []
    stack: list[dict[str, object]] = []

    for line_no, line in enumerate(dump, 1):
        match = LINE_RE.match(line)
        if not match:
            continue

        indent = len(match.group(1))
        label = match.group(2)
        file_name = match.group(3)
        start = int(match.group(4)) if match.group(4) is not None else None
        end = int(match.group(5)) if match.group(5) is not None else None

        while stack and stack[-1]["indent"] >= indent:
            stack.pop()

        ancestor = next(
            (
                node
                for node in reversed(stack)
                if node["file"] is not None and node["start"] is not None and node["end"] is not None
            ),
            None,
        )

        if file_name is not None and start is not None and end is not None:
            source = load_file(root, file_name, file_cache)
            if not (0 <= start < end <= len(source)):
                errors.append(f"line {line_no}: invalid range: {line}")
            else:
                snippet = source[start:end]
                if not snippet.strip():
                    errors.append(f"line {line_no}: blank span: {line}")

                if (
                    ancestor is not None
                    and ancestor["file"] == file_name
                    and not (ancestor["start"] <= start <= end <= ancestor["end"])
                ):
                    errors.append(
                        "line "
                        f"{line_no}: child escapes parent:\n  child: {line}\n  parent: {ancestor['line']}"
                    )

                keyword = KEYWORD_MAP.get(label)
                if keyword is not None and keyword not in snippet:
                    errors.append(
                        f"line {line_no}: span does not contain expected keyword {keyword!r}: {line}"
                    )

                if label == "Identifier":
                    quoted = re.search(r'"([^"]+)"', line)
                    if quoted and quoted.group(1) not in snippet:
                        errors.append(
                            f"line {line_no}: identifier text missing from span {snippet!r}: {line}"
                        )
                elif label in IDENTIFIER_LABELS:
                    token = re.search(r"\s([A-Za-z_][A-Za-z0-9_]*)$", line)
                    if token and token.group(1) not in snippet:
                        errors.append(
                            f"line {line_no}: token text missing from span {snippet!r}: {line}"
                        )

        stack.append(
            {
                "indent": indent,
                "file": file_name,
                "start": start,
                "end": end,
                "line": line,
            }
        )

    if errors:
        for error in errors[:20]:
            print(error)
        if len(errors) > 20:
            print(f"... and {len(errors) - 20} more")
        return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
