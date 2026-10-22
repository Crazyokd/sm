import re
import unittest
from pathlib import Path


ROOT = Path(__file__).parents[1]
PACKAGE = ROOT
SKILL = ROOT / "SKILL.md"


class DistillToSkillsTest(unittest.TestCase):
    def test_package_is_standalone_and_agent_neutral(self) -> None:
        self.assertTrue(SKILL.exists(), f"missing standalone Skill: {SKILL}")
        self.assertEqual(PACKAGE.name, "distill-to-skills")

        content = SKILL.read_text(encoding="utf-8")
        match = re.match(r"^---\n(.*?)\n---\n", content, re.DOTALL)
        self.assertIsNotNone(match)

        keys = {
            line.split(":", 1)[0]
            for line in match.group(1).splitlines()
            if line and not line.startswith(" ")
        }
        self.assertEqual(keys, {"name", "description"})
        self.assertIn("\ndescription: Use when ", f"\n{match.group(1)}")

        for required in (
            "active conversation",
            "source of truth",
            "no-op",
            "patch",
            "create",
            "unverified",
            "relative links",
        ):
            self.assertIn(required, content)

        for forbidden in (
            "skill_manage",
            "Curator",
            "Hermes Memory",
            "~/.hermes",
            "/home/",
        ):
            self.assertNotIn(forbidden, content)


if __name__ == "__main__":
    unittest.main()
