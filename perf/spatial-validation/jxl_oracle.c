/* Test-only independent libjxl frame/layer discovery. No file is encoded here. */
#include <inttypes.h>
#include <jxl/decode.h>
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv) {
  if (argc != 3) return 2;
  FILE *input = fopen(argv[1], "rb");
  if (!input || fseek(input, 0, SEEK_END)) return 3;
  long length = ftell(input);
  if (length < 1 || length > 64 * 1024 * 1024 || fseek(input, 0, SEEK_SET)) return 4;
  unsigned char *bytes = malloc((size_t)length);
  if (!bytes || fread(bytes, 1, (size_t)length, input) != (size_t)length) return 5;
  fclose(input);
  JxlDecoder *decoder = JxlDecoderCreate(NULL);
  if (!decoder) return 6;
  int rc = 7;
  if (JxlDecoderSetCoalescing(decoder, argv[2][0] == '1') != JXL_DEC_SUCCESS ||
      JxlDecoderSetKeepOrientation(decoder, JXL_TRUE) != JXL_DEC_SUCCESS ||
      JxlDecoderSubscribeEvents(decoder, JXL_DEC_BASIC_INFO | JXL_DEC_FRAME) != JXL_DEC_SUCCESS ||
      JxlDecoderSetInput(decoder, bytes, (size_t)length) != JXL_DEC_SUCCESS) goto end;
  JxlDecoderCloseInput(decoder);
  printf("version %" PRIu32 "\n", JxlDecoderVersion());
  for (unsigned events = 0; events < 4096; events++) {
    JxlDecoderStatus status = JxlDecoderProcessInput(decoder);
    if (status == JXL_DEC_SUCCESS) { rc = 0; break; }
    if (status == JXL_DEC_BASIC_INFO) {
      JxlBasicInfo info;
      if (JxlDecoderGetBasicInfo(decoder, &info) != JXL_DEC_SUCCESS) break;
      printf("canvas %" PRIu32 " %" PRIu32 "\n", info.xsize, info.ysize);
    } else if (status == JXL_DEC_FRAME) {
      JxlFrameHeader frame;
      if (JxlDecoderGetFrameHeader(decoder, &frame) != JXL_DEC_SUCCESS) break;
      printf("frame %" PRIu32 " %" PRIu32 " %" PRIu32 " %" PRId32 " %" PRId32 "\n",
             frame.duration, frame.layer_info.xsize, frame.layer_info.ysize,
             frame.layer_info.crop_x0, frame.layer_info.crop_y0);
    } else { fprintf(stderr, "unexpected libjxl status %d\n", status); break; }
  }
end:
  JxlDecoderDestroy(decoder);
  free(bytes);
  return rc;
}
