# 2026-10-06. Hot reload of scene content
Status: accepted
Summary: re2 and web serve --watch re-validate a saved scene or game and apply it in place, keeping the player; an invalid save changes nothing and shows validate's words

## Context
What forced a decision: the problem, what was tried, what it cost.

## Decision
What we do. Link code by symbol name (`red_engine2 src show <symbol>`), not by line number.

## Consequences
What gets easier and what gets harder; how to undo it.
