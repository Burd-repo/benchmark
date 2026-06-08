# Burd Benchmark Agent Instructions

This repository contains the Burd Agent, Burd Benchmark and benchmark visual interface.

Before implementing or modifying any visual interface, benchmark dashboard, report screen, TUI, web dashboard, UI component, visual token, color, typography, spacing, icon, chart, card, table or layout, read and follow:

SKILL.md

The design system in `SKILL.md` is mandatory for all visual work.

Do not invent a new visual identity.
Do not use generic SaaS templates.
Do not create UI that conflicts with the Burd design system.
Do not modify the institutional landing page from the main Burd repo.

If `SKILL.md` is missing, stop and ask for it before implementing visual work.

For benchmark/agent work, preserve the goal of the repository:

* reuse/adapt llmfit as the technical foundation whenever possible;
* add Burd-specific provider validation, scoring, reporting, antifraud structure and benchmark UI;
* keep license/copyright notices for any llmfit code or logic reused.
