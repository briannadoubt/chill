#ifndef CHILL_COMPRESSION_H
#define CHILL_COMPRESSION_H

#include <stddef.h>
#include <stdint.h>

/// Returns the maximum gzip output size for `input_size`, or zero on error.
size_t chill_gzip_bound(size_t input_size);

/// Compresses one complete gzip member. `output_size` is capacity on entry and
/// bytes written on success. Returns zero on success.
int chill_gzip_compress(
  const uint8_t *input,
  size_t input_size,
  uint8_t *output,
  size_t *output_size
);

/// Decompresses one complete gzip member into an exact caller-owned buffer.
/// `output_size` is capacity on entry and bytes written on success.
int chill_gzip_decompress(
  const uint8_t *input,
  size_t input_size,
  uint8_t *output,
  size_t *output_size
);

#endif
