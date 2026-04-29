# Project Skills

This directory is the canonical project-local source for shared skills.

Compatibility notes:
- Some agents support extra frontmatter fields such as tool permissions or invocability flags. Agents that do not support those fields should ignore them.
- This repository also exposes the same skills through `.claude/skills` via a symlink for Claude-compatible tooling.
- OpenCode can discover both `.agents/skills` and `.claude/skills`. With the current layout, that may expose duplicate skill names in OpenCode. This is a known limitation of the chosen compatibility setup.
