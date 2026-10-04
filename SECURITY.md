# Security

`dunnenote-format` reads files that may come from anyone: notebooks, and the CSV, JSON, `.ics`
and picture files it imports. A problem in how it, or the specification, handles a hostile file
is a security problem. [SPEC.md §15](SPEC.md#15-security-considerations) lists what readers and
writers must guard against.

## Reporting a problem

Please report it privately, through this repository's **Security** tab → **Report a
vulnerability** (GitHub's private vulnerability reporting), not in a public issue. Include:

- what you did, ideally with a small notebook or file that shows the problem;
- what happened, and what you expected;
- the version (`dnfmt --version`, or the git commit).

We will acknowledge your report, keep you informed while we work on it, and credit you in the
changelog unless you would rather we did not. Please give us a reasonable time to release a fix
before you disclose the problem publicly.

Problems in the DunneNote app itself are out of scope here: report those to Dunne, Corp.
through DunneNote's own support channels.

## Supported versions

Fixes go into the latest release. Until 1.0, earlier 0.x releases are not patched; update to
the latest one.
