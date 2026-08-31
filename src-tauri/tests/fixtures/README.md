# Test fixtures

**`Empty.pst`** — a genuine Unicode Outlook PST, taken from Microsoft's
[`outlook-pst-rs`](https://github.com/microsoft/outlook-pst-rs) repository, where it is the
example file for the reference implementation. MIT licensed.

It is here because it is the only PST this project has that something other than this project
produced. A fixture we wrote ourselves would only prove that the reader can read the writer.

It contains the standard folder tree and **no mail**, which bounds what `pst_gate.rs` can claim:
that the store opens and the hierarchy is walked, and nothing about extracting a message.
