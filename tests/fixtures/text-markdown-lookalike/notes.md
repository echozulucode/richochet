# text-markdown-lookalike

**Source:** hand-written (`source.txt`), 2026-09-10.

**What this proves:** Plain text containing Markdown metacharacters. Converting to Markdown MUST escape them, or the text silently changes meaning — this is the escaping bug that is invisible without a fixture.
