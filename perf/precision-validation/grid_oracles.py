"""Independent arithmetic controls for canonical and codec-specific code grids.

These are code-grid witnesses, not codec tests or performance measurements.
The named AVIF functions mirror pinned convert.rs formulas only; native tests
must independently exercise the actual encode/decode path.
"""
import unittest


def full_scale(value, source_bits, target_bits):
    source_max = (1 << source_bits) - 1
    target_max = (1 << target_bits) - 1
    return (value * target_max + source_max // 2) // source_max


def full_member(value, storage_bits, coded_bits):
    return full_scale(full_scale(value, storage_bits, coded_bits),
                      coded_bits, storage_bits) == value


def narrow_member(value, storage_bits, coded_bits):
    if not 8 <= coded_bits <= storage_bits:
        return False
    return value & ((1 << (storage_bits - coded_bits)) - 1) == 0


def avif_expand(code, depth):
    shift = 16 - depth
    return code if shift == 0 else (code << shift) | (code >> (depth - shift))


class GridControls(unittest.TestCase):
    def test_canonical_ten_is_not_avif_replication(self):
        canonical = full_scale(9, 10, 16)
        self.assertEqual(canonical, 577)
        self.assertTrue(full_member(canonical, 16, 10))
        self.assertEqual(canonical >> 6, 9)
        self.assertEqual(avif_expand(canonical >> 6, 10), 576)
        self.assertFalse(full_member(576, 16, 10))

    def test_more_full_range_bits_do_not_preserve_u16_eight_bit_grid(self):
        canonical = full_scale(1, 8, 16)
        self.assertEqual(canonical, 257)
        self.assertTrue(full_member(canonical, 16, 8))
        self.assertFalse(full_member(canonical, 16, 10))
        self.assertTrue(full_member(canonical, 16, 16))

    def test_narrow_colors_and_full_alpha_are_different_grids(self):
        self.assertTrue(narrow_member(4096, 16, 10))
        self.assertTrue(narrow_member(4096, 16, 12))
        self.assertFalse(narrow_member(4097, 16, 10))
        self.assertTrue(full_member(65535, 16, 10))
        self.assertFalse(narrow_member(65535, 16, 10))
        self.assertTrue(narrow_member(32768, 16, 10))
        self.assertFalse(full_member(32768, 16, 10))

    def test_row_padding_is_not_a_sample(self):
        # Two active RGB pixels, each followed by a padding word off the grid.
        words = [0, 257, 65535, 1, 514, 771, 1028, 1]
        for start in [0, 4]:
            self.assertTrue(all(full_member(v, 16, 8) for v in words[start:start+3]))
            self.assertFalse(full_member(words[start+3], 16, 8))


if __name__ == "__main__":
    unittest.main()
