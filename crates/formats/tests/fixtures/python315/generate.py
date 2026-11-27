"""Regenerate original compiler/serializer fixtures with CPython 3.15.2.

Run this file with that interpreter. No compiled fixture code is executed.
Original source and wrapper by Afloat16, under MIT OR Apache-2.0.
"""
import hashlib
import json
import marshal
import os
from pathlib import Path
import py_compile
import sys
import tempfile
import types

SOURCE = '''"""An original format-inspection fixture, never executed."""
TITLE = "inspection π"

def select_tail(values):
    return values[1:-1]

class Widget:
    def render(self, label="frame"):
        return f"{TITLE}: {label}"

async def probe():
    return "async route"

def nesting():
    def inner(item):
        return {"message": "retained", "payload": item}
    return inner
'''
UPSTREAM = "5e58eaad7c58df58dfa8ea549c87996e01dd1077"


def facts(code):
    """Observe code metadata and text without interpreting its instructions."""
    strings, children = [], []

    def constant(value):
        if isinstance(value, str):
            strings.append(value)
        elif isinstance(value, types.CodeType):
            children.append(facts(value))
        elif isinstance(value, (tuple, list, set, frozenset)):
            for item in value:
                constant(item)
        elif isinstance(value, (dict, frozendict)):
            for key, item in value.items():
                constant(key)
                constant(item)
        elif isinstance(value, slice):
            for item in (value.start, value.stop, value.step):
                constant(item)

    constant(code.co_consts)
    return {"name": code.co_name, "qualname": code.co_qualname,
            "filename": code.co_filename, "first_line": code.co_firstlineno,
            "arg_count": code.co_argcount, "posonly_arg_count": code.co_posonlyargcount,
            "kwonly_arg_count": code.co_kwonlyargcount, "flags": code.co_flags,
            "bytecode_size": len(code.co_code), "names": list(code.co_names),
            "strings": strings, "children": children}


def main():
    if sys.implementation.name != "cpython" or sys.version_info[:3] != (3, 15, 2):
        raise SystemExit("regenerate with CPython 3.15.2")
    files, expected = [], {}
    with tempfile.TemporaryDirectory(prefix="craft-python-fixture-") as folder:
        source = Path(folder) / "original.py"
        source.write_text(SOURCE, encoding="utf-8", newline="\n")
        os.utime(source, (1_700_000_000, 1_700_000_000))
        for mode in py_compile.PycInvalidationMode:
            path = Path(folder) / (mode.name + ".pyc")
            py_compile.compile(str(source), cfile=str(path), dfile="craft_original_fixture.py",
                               doraise=True, invalidation_mode=mode, optimize=0)
            data = path.read_bytes()
            code = marshal.loads(data[16:])
            expected["ordinary"] = facts(code)
            files.append({"case": mode.name.lower(), "oracle": "ordinary", "invalidation": mode.name.lower(),
                          "magic": int.from_bytes(data[:2], "little"),
                          "sha256": hashlib.sha256(data).hexdigest(), "hex": data.hex()})

        frozen = frozendict({"frozen key": "frozen value", "range": slice(1, 5, 2)})
        code = marshal.loads(bytes.fromhex(files[0]["hex"])[16:])
        extended = code.replace(co_consts=code.co_consts + (frozen, frozen))
        encoded = marshal.dumps(extended)
        decoded = marshal.loads(encoded)
        assert decoded.co_consts[-1] is decoded.co_consts[-2]
        assert decoded.co_consts[-1] == frozen
        expected["frozen_constants"] = facts(decoded)
        data = bytes.fromhex(files[0]["hex"])[:16] + encoded
        files.append({"case": "frozen_constants", "oracle": "frozen_constants", "invalidation": "timestamp",
                      "magic": int.from_bytes(data[:2], "little"),
                      "sha256": hashlib.sha256(data).hexdigest(), "hex": data.hex()})
        aliases = marshal.dumps((frozen, frozen))
        read_aliases = marshal.loads(aliases)
        assert read_aliases[0] is read_aliases[1]

    result = {"generator": "CPython 3.15.2 py_compile and marshal", "upstream_commit": UPSTREAM,
              "source_sha256": hashlib.sha256(SOURCE.encode()).hexdigest(),
              "frozen_constants_provenance": "CodeType.replace extends a compiled code object; official marshal writes and reads it. This is not a compiler-folded frozen-dictionary literal.",
              "expected_code": expected, "files": files, "frozen_aliases_hex": aliases.hex()}
    Path(__file__).with_name("fixtures.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main()
