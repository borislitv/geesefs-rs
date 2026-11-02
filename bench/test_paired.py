import unittest

import paired


class PairedAssessmentTests(unittest.TestCase):
    def test_one_adverse_pair_does_not_veto_a_reproducible_improvement(self):
        result = paired.assess([0.7] * 7 + [1.01], [0.99, 1.01] * 4)
        self.assertEqual(result["status"], "improved")
        self.assertLess(result["ci95"][1], 1)

    def test_slowdown_larger_than_independent_control_noise_is_a_regression(self):
        result = paired.assess([1.2] * 8, [0.99, 1.01] * 4)
        self.assertEqual(result["status"], "regressed")

    def test_control_noise_permits_non_inferiority_but_not_superiority(self):
        result = paired.assess([1.001] * 8, [0.98, 1.02] * 4, margin=1.01)
        self.assertEqual(result["status"], "non_inferior")
        self.assertGreater(result["noise_ratio"], 1.001)

    def test_wide_noise_cannot_authorize_a_practically_unacceptable_loss(self):
        self.assertEqual(paired.assess([2.0] * 8, [0.2, 5.0] * 4)["status"], "within_noise")
        result = paired.assess([2.0] * 8, [0.2, 5.0] * 4, margin=1.05)
        self.assertEqual(result["status"], "regressed")

    def test_missing_practical_margin_is_not_proof_of_non_inferiority(self):
        result = paired.assess([1.001] * 8, [0.98, 1.02] * 4)
        self.assertEqual(result["status"], "within_noise")

    def test_invalid_practical_margin_is_rejected(self):
        for value in [True, 0.99, float("nan"), float("inf"), "1.05"]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                paired.assess([1.0] * 8, [1.0] * 8, margin=value)

    def test_four_pairs_are_descriptive_even_when_all_favor_the_candidate(self):
        result = paired.assess([0.5] * 4, [0.99, 1.01] * 4)
        self.assertEqual(result["status"], "insufficient_pairs")
        self.assertEqual(result["sign_p_two_sided"], 0.125)

    def test_missing_or_biased_controls_cannot_authorize_a_change(self):
        self.assertEqual(paired.assess([0.8] * 8, [])["status"], "insufficient_controls")
        self.assertEqual(paired.assess([0.8] * 8, [1.2] * 8)["status"], "biased_controls")

    def test_shared_host_variance_can_leave_the_result_inconclusive(self):
        result = paired.assess([0.8, 1.3] * 4, [0.999, 1.001] * 4)
        self.assertEqual(result["status"], "inconclusive")

    def test_invalid_ratios_are_not_missing_values_or_silent_wins(self):
        for value in [0, -1, True, float("nan"), float("inf"), None]:
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "finite positive"):
                paired.assess([value] * 8, [1.0] * 8)

    def test_exactly_identical_controls_are_unbiased(self):
        self.assertEqual(paired.assess([0.7] * 8, [1.0] * 8)["status"], "improved")

    def test_unchanged_cost_is_not_an_improvement(self):
        result = paired.assess([1.0] * 8, [0.98, 1.02] * 4)
        self.assertEqual(result["ci95"], [1.0, 1.0])
        self.assertEqual(result["status"], "within_noise")

    def test_unit_margin_is_a_valid_no_extra_cost_requirement(self):
        self.assertEqual(paired.assess([1.0] * 8, [0.98, 1.02] * 4, margin=1)["status"], "non_inferior")

    def test_noise_factor_is_the_95th_percentile_absolute_control_deviation(self):
        # Seven exact controls and one 1.5x outlier: rank ceil(0.95 * 8) is the outlier.
        self.assertAlmostEqual(paired.assess([1.0] * 8, [1.0] * 7 + [1.5])["noise_ratio"], 1.5)

    def test_sign_probability_excludes_ties_and_counts_both_tails(self):
        self.assertEqual(paired.assess([1.0] * 8, [])["sign_p_two_sided"], 1.0)
        self.assertEqual(paired.assess([0.5] * 4 + [1.0] * 4, [])["sign_p_two_sided"], 0.125)
        # One opposite sign among eight: 2 * (C(8,0) + C(8,1)) / 2**8.
        self.assertEqual(paired.assess([0.5] * 7 + [2.0], [])["sign_p_two_sided"], 18 / 256)

    def test_resampling_is_reproducible_and_does_not_pseudoreplicate_operations(self):
        ratios = [0.8, 0.9, 1.0, 1.1] * 2
        first = paired.assess(ratios, [0.99, 1.01] * 4)
        self.assertEqual(first, paired.assess(ratios, [0.99, 1.01] * 4))
        self.assertEqual(first["pairs"], 8)
