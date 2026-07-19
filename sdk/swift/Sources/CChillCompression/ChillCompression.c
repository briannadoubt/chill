#include "ChillCompression.h"

#include <limits.h>
#include <string.h>
#include <zlib.h>

static int chill_assign_input(z_stream *stream, const uint8_t *input, size_t size) {
  if (size > UINT_MAX) {
    return Z_BUF_ERROR;
  }
  stream->next_in = (Bytef *)input;
  stream->avail_in = (uInt)size;
  return Z_OK;
}

static int chill_assign_output(z_stream *stream, uint8_t *output, size_t size) {
  if (size > UINT_MAX) {
    return Z_BUF_ERROR;
  }
  stream->next_out = output;
  stream->avail_out = (uInt)size;
  return Z_OK;
}

size_t chill_gzip_bound(size_t input_size) {
  if (input_size > UINT_MAX) {
    return 0;
  }
  z_stream stream;
  memset(&stream, 0, sizeof(stream));
  if (deflateInit2(
        &stream,
        Z_DEFAULT_COMPRESSION,
        Z_DEFLATED,
        15 + 16,
        8,
        Z_DEFAULT_STRATEGY
      ) != Z_OK) {
    return 0;
  }
  uLong bound = deflateBound(&stream, (uLong)input_size);
  deflateEnd(&stream);
  return (size_t)bound;
}

int chill_gzip_compress(
  const uint8_t *input,
  size_t input_size,
  uint8_t *output,
  size_t *output_size
) {
  if (output_size == NULL) {
    return Z_STREAM_ERROR;
  }
  z_stream stream;
  memset(&stream, 0, sizeof(stream));
  int status = deflateInit2(
    &stream,
    Z_DEFAULT_COMPRESSION,
    Z_DEFLATED,
    15 + 16,
    8,
    Z_DEFAULT_STRATEGY
  );
  if (status != Z_OK) {
    return status;
  }
  status = chill_assign_input(&stream, input, input_size);
  if (status == Z_OK) {
    status = chill_assign_output(&stream, output, *output_size);
  }
  if (status == Z_OK) {
    status = deflate(&stream, Z_FINISH);
    if (status == Z_STREAM_END) {
      *output_size = (size_t)stream.total_out;
      status = Z_OK;
    }
  }
  deflateEnd(&stream);
  return status;
}

int chill_gzip_decompress(
  const uint8_t *input,
  size_t input_size,
  uint8_t *output,
  size_t *output_size
) {
  if (output_size == NULL) {
    return Z_STREAM_ERROR;
  }
  z_stream stream;
  memset(&stream, 0, sizeof(stream));
  int status = inflateInit2(&stream, 15 + 16);
  if (status != Z_OK) {
    return status;
  }
  status = chill_assign_input(&stream, input, input_size);
  if (status == Z_OK) {
    status = chill_assign_output(&stream, output, *output_size);
  }
  if (status == Z_OK) {
    status = inflate(&stream, Z_FINISH);
    if (status == Z_STREAM_END) {
      *output_size = (size_t)stream.total_out;
      status = Z_OK;
    }
  }
  inflateEnd(&stream);
  return status;
}
