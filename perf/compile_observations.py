"""Describe observation placement without discarding or waiving quality flags."""


def summarize(invocations):
    result = {"boundary_only_pairs": 0, "other_layouts": 0,
              "before_cpu_flags": 0, "after_cpu_flags": 0,
              "before_heavy_flags": 0, "after_heavy_flags": 0,
              "during_observations": 0}
    for invocation in invocations:
        observations = invocation["observations"]
        result["during_observations"] += sum(o.get("phase") == "during" for o in observations)
        # Only the exact historical pair layout identifies these positions.
        # Unknown, missing or extended layouts must not be silently classified.
        if len(observations) != 2 or any(o.get("phase") != "boundary" for o in observations):
            result["other_layouts"] += 1
            continue
        result["boundary_only_pairs"] += 1
        for position, observation in zip(("before", "after"), observations):
            result[f"{position}_cpu_flags"] += observation["cpu_load"] > .05
            result[f"{position}_heavy_flags"] += observation["heavy_process_count"] > 0
    return result
