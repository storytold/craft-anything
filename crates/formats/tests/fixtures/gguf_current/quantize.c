/* Fixture-only driver for the pinned upstream reference quantizers; not app code. */
#include "ggml-quants.h"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv) {
    if (argc != 3) return 2;
    if (strcmp(argv[1], "traits") == 0) {
        const enum ggml_type kind = (enum ggml_type) strtol(argv[2], NULL, 10);
        if (kind < 0 || kind >= GGML_TYPE_COUNT) return 2;
        printf("{\"block\":%lld,\"bytes\":%zu,\"quantized\":%s}\n",
               (long long) ggml_blck_size(kind), ggml_type_size(kind),
               ggml_is_quantized(kind) ? "true" : "false");
        return 0;
    }
    const size_t count = strtoul(argv[2], NULL, 10);
    if (!count || count > 4096) return 2;
    float *input = malloc(count * sizeof(float));
    if (!input || fread(input, sizeof(float), count, stdin) != count) return 3;
    if (strcmp(argv[1], "NVFP4") == 0) {
        if (count % QK_NVFP4) return 4;
        const size_t bytes = count / QK_NVFP4 * sizeof(block_nvfp4);
        block_nvfp4 *output = malloc(bytes);
        if (!output) return 3;
        quantize_row_nvfp4_ref(input, output, (int64_t) count);
        if (fwrite(output, 1, bytes, stdout) != bytes) return 5;
        free(output);
    } else if (strcmp(argv[1], "F8_E4M3") == 0) {
        const size_t bytes = count * sizeof(ggml_fp8_e4m3_t);
        ggml_fp8_e4m3_t *output = malloc(bytes);
        if (!output) return 3;
        quantize_row_f8_e4m3_ref(input, output, (int64_t) count);
        if (fwrite(output, 1, bytes, stdout) != bytes) return 5;
        free(output);
    } else {
        return 4;
    }
    free(input);
    return 0;
}
