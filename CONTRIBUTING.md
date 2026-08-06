# Contributing

## Building

```bash
cargo build --workspace
cargo test --workspace
```

Before opening a pull request, run what CI runs:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

## Adding a preset

Presets live in `engine/src/bypass.rs`. Add a constructor, list the name in
`preset_names`, and wire it into `preset()`. The test that every preset splits inside the
hostname runs against that list automatically, so a new preset that fails to split will
fail the build.

If you add one for an ISP, say which one and roughly when you tested it. Blocking changes.

## Reporting that a preset stopped working

Run this and paste the output:

```bash
turkeydpi doctor --json
```

Include your ISP and city. `doctor` output is far more useful than "discord doesn't work".

## Style

- No comments. Name things so they do not need one.
- Keep the byte-stream invariant: any transform must reassemble to exactly the input
  bytes. `test_transforms_preserve_byte_stream` guards this.
- New parsing code that touches network input needs a test that feeds it truncated and
  mutated bytes.
