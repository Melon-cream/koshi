# Contributing

## Branches

- Work on a branch other than `main`. Create one before you start.
- `main` is protected: a change only lands through a pull request. Pushing a branch is not enough,
  so open the pull request in the same step.
- `main` carries no commits until a release. Leave it alone until you are asked to release.

## Build

Do not install a Rust toolchain on the host. Build inside the container.

```sh
docker build -f Dockerfile.dev -t koshi-dev .
./scripts/dev.sh cargo build
./scripts/dev.sh cargo test
```

With a host toolchain, `cargo build` works as usual. `./scripts/release.sh` builds the
static binary into `dist/koshi`.

## Tests

The acceptance tests run the real binary on a PTY and turn the ANSI output back into a
screen.

```sh
./tests/e2e/run.sh
```

`tests/e2e/` is the specification side. Do not rewrite it to match the implementation.

## Dependencies

- Ask before adding a crate.
- Use `alacritty_terminal` for the VT and `portable-pty` for the PTY. Do not replace them
  with a local implementation.
- Do not add outbound network access or a TCP listener. IPC is a Unix domain socket only.

## Code

- Comments are in English and say only what the code cannot.
- Split a file when it passes 400 lines.
- Do not copy code or prose from other implementations. Reference behaviour only.
