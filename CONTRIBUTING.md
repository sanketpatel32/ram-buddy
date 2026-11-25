# Contributing to RAM Buddy

## Report a bug or request a feature

Search [GitHub Issues](https://github.com/sanketpatel32/ram-buddy/issues) first. For a bug, include your Windows version, RAM Buddy version, the steps to reproduce it, and what you expected to happen. Screenshots of the tray or dialog can help. Remove personal information from screenshots before posting.

For a feature request, describe the problem you want to solve and how you would use the change.

## Make a change

1. Fork the repository and create a branch from `main`.
2. Use the [Windows build instructions](README.md#build-from-source). Aim to keep the tray app small and reuse native Windows APIs where practical.
3. Run the checks below and open a pull request against `main`. Describe what changed, how you checked it, and any Windows behavior that reviewers should try.

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

For process cleanup changes, also run the disposable-process smoke test. It temporarily allocates about 272 MiB to check that cleanup stops the heavy fixture and leaves the light one running.

```powershell
cargo test --locked stops_only_the_heavy_test_process -- --ignored
```

For tray changes, run the release build on Windows and check the animation, tooltip, left-click preview, and right-click Exit command. Cancel the preview if it lists your own work.
