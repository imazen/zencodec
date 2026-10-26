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
  if (argc != 3 && argc != 4) return 2;
  int full = argc == 4 && !strcmp(argv[3], "full");
  FILE *input = fopen(argv[1], "rb");
  if (!input || fseek(input, 0, SEEK_END)) return 3;
  long length = ftell(input);
  if (length < 1 || length > 64 * 1024 * 1024 || fseek(input, 0, SEEK_SET)) return 4;
  unsigned char *bytes = malloc((size_t)length);
  if (!bytes || fread(bytes, 1, (size_t)length, input) != (size_t)length) return 5;
  fclose(input);
  JxlDecoder *decoder = JxlDecoderCreate(NULL);
  if (!decoder) return 6;
  void *color = NULL;
  void *extras[16] = {0};
  JxlBasicInfo basic = {0};
  unsigned decoded = 0;
  int rc = 7;
  if (JxlDecoderSetCoalescing(decoder, argv[2][0] == '1') != JXL_DEC_SUCCESS ||
      JxlDecoderSetKeepOrientation(decoder, JXL_TRUE) != JXL_DEC_SUCCESS ||
      JxlDecoderSubscribeEvents(decoder, JXL_DEC_BASIC_INFO | JXL_DEC_FRAME |
                                (full ? JXL_DEC_FULL_IMAGE : 0)) != JXL_DEC_SUCCESS ||
      JxlDecoderSetInput(decoder, bytes, (size_t)length) != JXL_DEC_SUCCESS) goto end;
  JxlDecoderCloseInput(decoder);
  printf("version %" PRIu32 "\n", JxlDecoderVersion());
  for (unsigned events = 0; events < 4096; events++) {
    JxlDecoderStatus status = JxlDecoderProcessInput(decoder);
    if (status == JXL_DEC_SUCCESS) { rc = full && !decoded ? 7 : 0; break; }
    if (status == JXL_DEC_BASIC_INFO) {
      JxlBasicInfo info;
      if (JxlDecoderGetBasicInfo(decoder, &info) != JXL_DEC_SUCCESS) break;
      basic = info;
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
        printf("extra_details %" PRIu32 " %u %" PRIu32 " %" PRIu32 " %" PRIu32
               " %" PRIu32 " %" PRIu32 " %" PRIu32 " %" PRIu32 " %" PRIu32 "\n",
               i, (unsigned)extra.type, extra.bits_per_sample, extra.exponent_bits_per_sample,
               extra.dim_shift, float_bits(extra.spot_color[0]), float_bits(extra.spot_color[1]),
               float_bits(extra.spot_color[2]), float_bits(extra.spot_color[3]), extra.cfa_channel);
      }
    } else if (status == JXL_DEC_FRAME) {
      JxlFrameHeader frame;
      if (JxlDecoderGetFrameHeader(decoder, &frame) != JXL_DEC_SUCCESS) break;
      printf("frame %" PRIu32 " %" PRIu32 " %" PRIu32 " %" PRId32 " %" PRId32 "\n",
             frame.duration, frame.layer_info.xsize, frame.layer_info.ysize,
             frame.layer_info.crop_x0, frame.layer_info.crop_y0);
      printf("timecode %" PRIu32 "\n", frame.timecode);
    } else if (status == JXL_DEC_NEED_IMAGE_OUT_BUFFER && full) {
      if (basic.num_extra_channels > 16) goto end;
      JxlPixelFormat format = {basic.num_color_channels, JXL_TYPE_FLOAT, JXL_NATIVE_ENDIAN, 0};
      size_t size = 0;
      if (JxlDecoderImageOutBufferSize(decoder, &format, &size) != JXL_DEC_SUCCESS ||
          size > 64 * 1024 * 1024) goto end;
      free(color);
      color = malloc(size);
      if (!color || JxlDecoderSetImageOutBuffer(decoder, &format, color, size) != JXL_DEC_SUCCESS) goto end;
      size_t total = size;
      format.num_channels = 1;
      for (uint32_t i = 0; i < basic.num_extra_channels; i++) {
        if (JxlDecoderExtraChannelBufferSize(decoder, &format, &size, i) != JXL_DEC_SUCCESS ||
            size > 64 * 1024 * 1024 || total + size > 128 * 1024 * 1024) goto end;
        total += size;
        free(extras[i]);
        extras[i] = malloc(size);
        if (!extras[i] || JxlDecoderSetExtraChannelBuffer(decoder, &format, extras[i], size, i) != JXL_DEC_SUCCESS) goto end;
      }
    } else if (status == JXL_DEC_FULL_IMAGE && full) {
      printf("decoded %u\n", decoded++);
    } else { fprintf(stderr, "unexpected libjxl status %d\n", status); break; }
  }
end:
  JxlDecoderDestroy(decoder);
  free(color);
  for (unsigned i = 0; i < 16; i++) free(extras[i]);
  free(bytes);
  return rc;
}
