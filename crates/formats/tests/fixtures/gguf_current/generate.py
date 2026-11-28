"""Regenerate fixtures.json from original weights with the official GGUF tools.

Run: python generate.py /path/to/llama.cpp
The checkout must be at REFERENCE below. Python needs NumPy and gguf-py's
dependencies; CMake and Linux C/C++ compilers build the official base library
for type traits and the two quantizers without Python encoders. None of these
tools is needed to run the Rust tests.
See ATTRIBUTION.md for ownership, licences and the pinned format references.
"""

import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

import numpy as np

REFERENCE = "a0677c341ba6bbfbbf74fa3c966f4dba74e1c830"
HERE = Path(__file__).resolve().parent
TYPES = ("TQ1_0", "TQ2_0", "MXFP4", "NVFP4", "Q1_0", "Q2_0", "F8_E4M3")
INPUT = (-1.0, -0.5, 0.0, 0.5, 1.0, 0.25, -0.25, 0.75)


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: python generate.py /path/to/llama.cpp")
    reference = Path(sys.argv[1]).resolve()
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=reference, text=True).strip()
    if actual != REFERENCE:
        raise SystemExit(f"expected llama.cpp {REFERENCE}, found {actual}")
    sys.path.insert(0, str(reference / "gguf-py"))
    import gguf

    fixtures = []
    with tempfile.TemporaryDirectory() as temporary:
        temporary = Path(temporary)
        encoder = temporary / "quantize"
        build = temporary / "ggml-build"
        project = temporary / "project"
        project.mkdir()
        (project / "CMakeLists.txt").write_text(
            'cmake_minimum_required(VERSION 3.14)\nproject(gguf_fixtures C CXX)\n'
            'add_subdirectory("${GGML_SOURCE}" ggml)\n'
        )
        subprocess.run([
            "cmake", "-S", str(project), "-B", str(build), "-DGGML_SOURCE=" + str(reference / "ggml"),
            "-DBUILD_SHARED_LIBS=OFF", "-DGGML_CPU=OFF", "-DGGML_OPENMP=OFF",
            "-DGGML_BUILD_TESTS=OFF", "-DGGML_BUILD_EXAMPLES=OFF", "-DCMAKE_BUILD_TYPE=Release",
        ], check=True)
        subprocess.run(["cmake", "--build", str(build), "--target", "ggml-base", "--parallel", "1"], check=True)
        libraries = list(build.rglob("libggml-base.a"))
        assert len(libraries) == 1, libraries
        driver = temporary / "quantize.o"
        subprocess.run([
            "cc", "-std=c11", "-D_GNU_SOURCE", "-O1",
            "-I" + str(reference / "ggml/include"), "-I" + str(reference / "ggml/src"),
            "-c", str(HERE / "quantize.c"), "-o", str(driver),
        ], check=True)
        subprocess.run(["c++", str(driver), str(libraries[0]), "-lm", "-pthread", "-ldl", "-o", str(encoder)], check=True)
        for label in TYPES:
            kind = getattr(gguf.GGMLQuantizationType, label)
            block, packed_bytes = gguf.GGML_QUANT_SIZES[kind]
            traits = json.loads(subprocess.check_output([str(encoder), "traits", str(int(kind))]))
            assert (traits["block"], traits["bytes"]) == (block, packed_bytes)
            weights = np.resize(np.array(INPUT, dtype=np.float32), (3, 2 * block))
            if label in {"NVFP4", "F8_E4M3"}:
                raw = subprocess.check_output([str(encoder), label, str(weights.size)], input=weights.tobytes())
                packed = np.frombuffer(raw, dtype=np.uint8).reshape(3, 2 * packed_bytes)
                quantizer = "ggml/src/ggml-quants.c reference quantizer"
            else:
                packed = gguf.quantize(weights, kind)
                quantizer = "gguf-py/gguf/quants.py quantizer"
            path = temporary / f"{label}.gguf"
            writer = gguf.GGUFWriter(str(path), "llama")
            writer.add_name("Craft Anything quantized-layout fixture")
            file_type = getattr(gguf.LlamaFileType, "MOSTLY_" + ("MXFP4_MOE" if label == "MXFP4" else label))
            writer.add_file_type(file_type)
            writer.add_tensor("audit.weight", packed, raw_dtype=kind)
            writer.write_header_to_file()
            writer.write_kv_data_to_file()
            writer.write_tensors_to_file()
            writer.close()

            reader = gguf.GGUFReader(str(path))
            tensor = reader.tensors[0]
            assert tensor.tensor_type == kind
            stored_file_type = gguf.LlamaFileType(reader.get_field("general.file_type").contents())
            assert stored_file_type == file_type
            assert tensor.shape.tolist() == list(reversed(weights.shape))
            assert tensor.n_bytes == packed.nbytes
            np.testing.assert_array_equal(tensor.data, packed)
            decoded = gguf.dequantize(tensor.data, kind)
            assert decoded.shape == weights.shape and np.isfinite(decoded).all()
            raw = path.read_bytes()
            truncated = temporary / "cut.gguf"
            truncated.write_bytes(raw[: tensor.data_offset + tensor.n_bytes - 1])
            try:
                gguf.GGUFReader(str(truncated))
            except ValueError:
                pass
            else:
                raise AssertionError(f"official reader accepted truncated {label} tensor")
            fixtures.append({
                "tensor_type": label, "type_id": int(kind), "file_type": stored_file_type.name,
                "file_type_id": int(stored_file_type), "shape": tensor.shape.tolist(),
                "quantized": traits["quantized"],
                "data_offset": tensor.data_offset, "data_bytes": tensor.n_bytes,
                "quantizer": quantizer, "sha256": hashlib.sha256(raw).hexdigest(),
                "hex": raw.hex(),
            })

    corpus = {
        "source": "https://github.com/ggml-org/llama.cpp/tree/" + REFERENCE,
        "generator_licence": "MIT", "fixture_licence": "MIT OR Apache-2.0",
        "author": "Afloat16", "inputs": {"values": INPUT, "rows": 3, "blocks_per_row": 2},
        "verification": "Official GGUFReader and dequantizer accept every fixture; GGUFReader rejects each one-byte-short tensor.",
        "fixtures": fixtures,
    }
    (HERE / "fixtures.json").write_text(json.dumps(corpus, indent=2) + "\n")
    print(f"wrote {len(fixtures)} official-writer fixtures to fixtures.json")


if __name__ == "__main__":
    main()
