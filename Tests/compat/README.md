# Compatibility canaries

`baseline-93883c1` identifies the repository immediately before the six API
stability steps. C/Swift snapshots live here and Rust snapshots live under the
crate's test fixtures. It is a development baseline for application version
0.1.6, not a claim that this commit was published as a release.

The C header, Swift tests and Rust `foundation.rs` are exact copies from commit
`93883c1`. `node.rs` is a small consumer written using that commit's public Node
methods and policy literals. Keep baseline calls unchanged: fixes go into the
current implementation. Add a new baseline when adopting a new contract rather
than rewriting the old consumer to make an incompatible change pass.

`cargo test --workspace --locked` compiles the frozen Rust consumers as external
integration tests alongside the supported facade tests. Rust compatibility here
means tested source compatibility, not a binary ABI or an exhaustive semver proof.
The intentional `non_exhaustive` enum migration is documented in
`docs/rust-api.md`; old exhaustive matches require a fallback arm.

After `cargo build --workspace --locked`, run:

```sh
python3 scripts/check-api.py
python3 scripts/test-c-api.py
python3 scripts/test-swift-api.py
python3 scripts/cli-smoke.py
```

The C checks verify exported symbols, function-pointer types against the frozen
header, and runtime behavior of old and new C11/C++17 consumers. They also compile
the iOS forwarding header. The Swift checks run the frozen and current consumers
against current SDK sources and the current core. The daemon smoke test checks
the socket transport. CI runs Rust/C/socket checks on Linux and macOS and the
Swift suite on macOS; release jobs additionally build the platform SDKs.

`crates/forwiroh/tests/fixtures/control-v1.json` freezes the minimum commands,
aliases, features and host symbols after these stability steps. Its unit test
permits additions and rejects removals or alias retargeting. Policy schema,
success and error fixtures live under `crates/forwiroh/tests/fixtures`, so Cargo
packages include them. The Swift test runner and release packager copy the same
success fixture into `Tests/Fixtures` in their generated packages.
Existing signature, ownership and return conventions stay fixed in C ABI v1.
Contract-breaking changes need a separate major interface and an explicit legacy
adapter, with their own tests; do not weaken these baselines to ship them.
