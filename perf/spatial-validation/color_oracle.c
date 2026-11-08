/* Independent test-only RGB/CMYK ICC conversion. Samples are native-endian f32. */
#include <errno.h>
#include <inttypes.h>
#include <lcms2.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static uint32_t dimension(const char *text) {
  char *end = NULL;
  errno = 0;
  unsigned long value = strtoul(text, &end, 10);
  if (errno || !text[0] || *end || value < 1 || value > 16384) return 0;
  return (uint32_t)value;
}

/* CMYK input is normalized float, optionally JXL's inverted polarity.
   Little CMS TYPE_CMYK_FLT instead consumes percentages of ink. */
static int cmyk(int argc, char **argv, int inverted) {
  if (argc != 8 || strcmp(argv[2], "-")) return 2;
  uint32_t width = dimension(argv[5]), height = dimension(argv[6]);
  if (!width || !height || strcmp(argv[7], "4") ||
      (uint64_t)width * height * 4 * sizeof(float) > 256 * 1024 * 1024)
    return 3;
  cmsHPROFILE source = cmsOpenProfileFromFile(argv[1], "r");
  cmsHPROFILE target = cmsCreate_sRGBProfile();
  if (!source || !target || cmsGetColorSpace(source) != cmsSigCmykData) return 4;
  cmsHTRANSFORM transform = cmsCreateTransform(
      source, TYPE_CMYK_FLT, target, TYPE_RGB_FLT,
      INTENT_RELATIVE_COLORIMETRIC,
      cmsFLAGS_BLACKPOINTCOMPENSATION | cmsFLAGS_HIGHRESPRECALC);
  if (!transform) return 5;
  FILE *input = fopen(argv[3], "rb"), *output = fopen(argv[4], "wbx");
  if (!input || !output) return 6;
  size_t samples = (size_t)width * 4;
  float *before = malloc(samples * sizeof(float));
  float *after = malloc((size_t)width * 3 * sizeof(float));
  if (!before || !after) return 7;
  for (uint32_t y = 0; y < height; y++) {
    if (fread(before, sizeof(float), samples, input) != samples) return 8;
    for (size_t i = 0; i < samples; i++)
      before[i] = inverted ? 100.0f - 100.0f * before[i] : 100.0f * before[i];
    cmsDoTransform(transform, before, after, width);
    if (fwrite(after, sizeof(float), (size_t)width * 3, output) != (size_t)width * 3)
      return 9;
  }
  if (fgetc(input) != EOF || ferror(input) || fclose(output)) return 10;
  fclose(input);
  free(before);
  free(after);
  cmsDeleteTransform(transform);
  cmsCloseProfile(source);
  cmsCloseProfile(target);
  printf("header %u runtime %u CMYK pixels %" PRIu64 " inverted %d\n",
         LCMS_VERSION, cmsGetEncodedCMMversion(), (uint64_t)width * height, inverted);
  return 0;
}

int main(int argc, char **argv) {
  if (argc == 9 && !strcmp(argv[1], "--cmyk-inverted")) return cmyk(argc - 1, argv + 1, 1);
  if (argc == 9 && !strcmp(argv[1], "--cmyk")) return cmyk(argc - 1, argv + 1, 0);
  if (argc == 2 && !strcmp(argv[1], "--version")) {
    printf("header %u runtime %u\n", LCMS_VERSION, cmsGetEncodedCMMversion());
    return 0;
  }
  if (argc != 8) return 2;
  uint32_t width = dimension(argv[5]), height = dimension(argv[6]);
  uint32_t channels = dimension(argv[7]);
  if (!width || !height || (channels != 3 && channels != 4) ||
      (uint64_t)width * height * channels * sizeof(float) > 256 * 1024 * 1024)
    return 3;
  cmsHPROFILE source = cmsOpenProfileFromFile(argv[1], "r");
  cmsHPROFILE target = cmsOpenProfileFromFile(argv[2], "r");
  if (!source || !target || cmsGetColorSpace(source) != cmsSigRgbData ||
      cmsGetColorSpace(target) != cmsSigRgbData) return 4;
  cmsUInt32Number format = channels == 3 ? TYPE_RGB_FLT : TYPE_RGBA_FLT;
  cmsHTRANSFORM transform = cmsCreateTransform(
      source, format, target, format, INTENT_RELATIVE_COLORIMETRIC,
      cmsFLAGS_NOOPTIMIZE | cmsFLAGS_COPY_ALPHA);
  if (!transform) return 5;
  FILE *input = fopen(argv[3], "rb"), *output = fopen(argv[4], "wbx");
  if (!input || !output) return 6;
  size_t samples = (size_t)width * channels;
  float *before = malloc(samples * sizeof(float));
  float *after = malloc(samples * sizeof(float));
  if (!before || !after) return 7;
  for (uint32_t y = 0; y < height; y++) {
    if (fread(before, sizeof(float), samples, input) != samples) return 8;
    cmsDoTransform(transform, before, after, width);
    if (fwrite(after, sizeof(float), samples, output) != samples) return 9;
  }
  if (fgetc(input) != EOF || ferror(input) || fclose(output)) return 10;
  fclose(input);
  free(before);
  free(after);
  cmsDeleteTransform(transform);
  cmsCloseProfile(source);
  cmsCloseProfile(target);
  printf("header %u runtime %u pixels %" PRIu64 "\n", LCMS_VERSION,
         cmsGetEncodedCMMversion(), (uint64_t)width * height);
  return 0;
}
