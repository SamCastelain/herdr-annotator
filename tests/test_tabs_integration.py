import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from tabs_integration import integrated_tabs


class TabsIntegrationTest(unittest.TestCase):
    def test_preserves_existing_tabs_and_sidebar_action(self):
        button = '.when(enabled.includes("changes"), el => el.child(panelButton("changes", "Changes", "mods/changes/changes.svg")))'
        source = "before\n" + button + "\nafter"
        result = integrated_tabs(source)
        self.assertIn("before\n", result)
        self.assertTrue(result.endswith("\nafter"))
        self.assertIn('panelButton("changes", "Changes", "mods/changes/changes.svg")', result)
        self.assertIn('id("panel-review-full")', result)
        self.assertIn("openReview(this, cx)", result)

    def test_unknown_or_ambiguous_source_is_refused(self):
        with self.assertRaises(ValueError):
            integrated_tabs("unfamiliar tabs implementation")
        button = '.when(enabled.includes("changes"), el => el.child(panelButton("changes", "Changes", "mods/changes/changes.svg")))'
        with self.assertRaises(ValueError):
            integrated_tabs(button + button)


if __name__ == "__main__":
    unittest.main()
