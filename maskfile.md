# rustow — task runner

All commands assume repo root as cwd. Use `$MASK` in scripts that chain other mask commands.

## build

> Build the debug binary

```bash
cargo build
```

## test

> Run all tests

```bash
cargo test
```

## lint

> Check formatting and lints

```bash
cargo fmt --check && cargo clippy --all-targets
```

## format [files]

> Format the project

```bash
eval "argv=(${files})"

if ((${#argv[@]} == 0)); then
  cargo fmt
else
  rustfmt --config-path rustfmt.toml "${argv[@]}"
fi
```

## clean

> Clean build artifacts

```bash
cargo clean
```

## install

> Install rustow into PATH (~/.cargo/bin)

```bash
cargo install --path .
```

## schema

> Regenerate schema.json at the repo root from the Config struct

```bash
cargo run --quiet --example generate_schema
```

### check

> Verify schema.json is up to date with the Config struct

```bash
cargo run --quiet --example generate_schema -- --check
```

## container

> Container commands.

### build

> Build the container

```bash
podman build \
  --build-arg USER_UID="$(id -u)" \
  --build-arg USER_GID="$(id -g)" \
  -t rustow-e2e \
  -f Dockerfile .
```

### run (command)

> Run a command on the container

**OPTIONS**

- interactive
  - flags: -i --interactive
  - type: boolean
  - desc: Make STDIN available to the contained process and allocate a pseudo-TTY for container

```bash
podman run --rm ${interactive:+'-it'} \
  --userns=keep-id \
  -v "$(pwd):/app" \
  -w /app \
  rustow-e2e \
  bash -c "${command}"
```

## e2e

> Run the end-to-end scenario in a Void Linux podman container

```bash
$MASK container build
$MASK container run "bash tests/e2e.sh"
```

## check

> Run lint + tests

```bash
$MASK lint && $MASK test
```

## release [target]

> Build linux release binaries into ./bin (needs `cross`); macOS/windows artifacts are built by the Release workflow

**OPTIONS**
* target
    * flags: -t --target
    * type: string
    * desc: Single rustup target to build (default: all linux targets)
    * choices: x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, x86_64-unknown-linux-musl, aarch64-unknown-linux-musl

```bash
if ! command -v cross &>/dev/null; then
  echo "error: 'cross' is required — install with: cargo install cross --git https://github.com/cross-rs/cross" >&2
  exit 1
fi

target_list=(x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-unknown-linux-musl aarch64-unknown-linux-musl)

if [[ -n "${target:-}" ]]; then
  target_list=("${target}")
fi

mkdir -p bin
for triple in "${target_list[@]}"; do
  case "${triple}" in
    x86_64-unknown-linux-gnu)   out="rustow-linux-gnu-x64" ;;
    aarch64-unknown-linux-gnu)  out="rustow-linux-gnu-arm64" ;;
    x86_64-unknown-linux-musl)  out="rustow-linux-musl-x64" ;;
    aarch64-unknown-linux-musl) out="rustow-linux-musl-arm64" ;;
    *) echo "unsupported target: ${triple}" >&2; exit 1 ;;
  esac
  echo "Building ${out} (${triple})"
  DOCKER_DEFAULT_PLATFORM=linux/amd64 cross build --release --target "${triple}"
  cp "target/${triple}/release/rustow" "bin/${out}"
done
```
