/* Test-only independent libjxl frame/layer discovery. No file is encoded here. */
#include <inttypes.h>
#include <jxl/decode.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static uint32_t float_bits(float value) {
  uint32_t bits;
  _Static_assert(sizeof(bits) == sizeof(value), "requires binary32 float");
  memcpy(&bits, &value, sizeof(bits));
  return bits;
}

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
      printf("orientation %u\n", (unsigned)info.orientation);
      printf("source_samples %" PRIu32 " %" PRIu32 "\n",
             info.bits_per_sample, info.exponent_bits_per_sample);
      printf("source_tone %" PRIu32 " %" PRIu32 " %u %" PRIu32 "\n",
             float_bits(info.intensity_target), float_bits(info.min_nits),
             (unsigned)info.relative_to_max_display, float_bits(info.linear_below));
      printf("source_display %" PRIu32 " %" PRIu32 "\n",
             info.intrinsic_xsize, info.intrinsic_ysize);
      printf("original_profile %u\n", (unsigned)info.uses_original_profile);
      printf("preview %u %" PRIu32 " %" PRIu32 "\n",
             (unsigned)info.have_preview, info.preview.xsize, info.preview.ysize);
      if (info.have_animation) {
        printf("animation %" PRIu32 " %" PRIu32 " %" PRIu32 " %u\n",
               info.animation.tps_numerator, info.animation.tps_denominator,
               info.animation.num_loops, (unsigned)info.animation.have_timecodes);
      }
      for (uint32_t i = 0; i < info.num_extra_channels; i++) {
        JxlExtraChannelInfo extra;
        if (JxlDecoderGetExtraChannelInfo(decoder, i, &extra) != JXL_DEC_SUCCESS) goto end;
        printf("extra %" PRIu32 " %u %" PRIu32 " %" PRIu32 " %u\n",
               i, (unsigned)extra.type, extra.bits_per_sample, extra.dim_shift,
               (unsigned)extra.alpha_premultiplied);
      }
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
