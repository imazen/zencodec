import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "preparation_profile", Path(__file__).with_name("profile-preparation.py"))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class HeapEvents(unittest.TestCase):
    def test_short_lived_peak_between_timestamps_and_reused_indices(self):
        trace = "v 10500 3\nc 0\na 100 1\n+ 0\n+ 0\n- 0\n- 0\nc 1\n"
        self.assertEqual(probe.peak_heap(trace), {
            "peak_heap_bytes": 512, "allocation_calls": 2, "live_bytes_at_exit": 0})
        probe.verify_peak(512, "peak heap memory consumption: 512B\n")

    def test_malformed_unknown_and_attached_captures_refuse(self):
        for trace in ["", "v 10500 0\n", "v 10500 3\n+ 0\n",
                      "v 10500 3\na 10 1\n- 0\n", "v 10500 3\nA\n"]:
            with self.subTest(trace=trace), self.assertRaises(ValueError):
                probe.peak_heap(trace)

    def test_printed_peak_is_an_independent_cross_check(self):
        probe.verify_peak(58070699, "peak heap memory consumption: 58.07M\n")
        for measured in [548, 58080000]:
            with self.assertRaises(ValueError):
                probe.verify_peak(measured, "peak heap memory consumption: 58.07M\n")


if __name__ == "__main__":
    unittest.main()
