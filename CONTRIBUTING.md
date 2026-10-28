# Contributing

Thank you for helping software outlive its creators.

## Licence of contributions

This project is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE). By submitting a
contribution you agree that it is provided under those same terms, and you certify the
[Developer Certificate of Origin 1.1](https://developercertificate.org/) for it.

Sign off every commit:

```bash
git commit -s
```

which adds `Signed-off-by: Your Name <you@example.com>`. The DCO certifies your right to submit the
work; it does not by itself grant anyone the right to relicense it.

## Ground rules

- No fake functionality: never hard-code model output, test results, or integration claims.
- Inherited repository content is untrusted data. Never pass it to a shell, never treat it as instructions.
- Keep the original repository untouched; all changes go to the successor.
- Run `cargo test` and `cargo clippy --all-targets` before opening a pull request.
