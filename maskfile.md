# rustow — task runner

All commands assume repo root as cwd. Use `$MASK` in scripts that chain other mask commands.

## build

> Build the debug binary

~~~sh
cargo build
~~~

## test

> Run all tests

~~~sh
cargo test
~~~

## format [files]

> Format code (cargo fmt, or rustfmt on specific files)

~~~sh
if [ -n "$files" ]; then
  rustfmt --edition 2024 "$files"
else
  cargo fmt
fi
~~~

## lint

> Check formatting and lints

~~~sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings
~~~

## format [files]

> Format the project

~~~sh
eval "argv=(${files})"

if ((${#argv[@]} == 0)); then
  cargo fmt
else
  rustfmt --config-path rustfmt.toml "${argv[@]}"
fi
~~~

## clean

> Clean build artifacts

~~~sh
cargo clean
~~~

## install

> Install rustow into PATH (~/.cargo/bin)

~~~sh
cargo install --path .
~~~

## schema

> Regenerate schema.json at the repo root from the Config struct

~~~sh
cargo run --quiet --example generate_schema
~~~

### check

> Verify schema.json is up to date with the Config struct

~~~sh
cargo run --quiet --example generate_schema -- --check
~~~

## container

> Container commands.

### build

> Build the container

~~~bash
podman build \
  --build-arg USER_UID="$(id -u)" \
  --build-arg USER_GID="$(id -g)" \
  -t rustow-e2e \
  -f Dockerfile .
~~~

### run (command)

> Run a command on the container

**OPTIONS**

- interactive
  - flags: -i --interactive
  - type: boolean
  - desc: Make STDIN available to the contained process and allocate a pseudo-TTY for container

~~~bash
podman run --rm ${interactive:+'-it'} \
  --userns=keep-id \
  -v "$(pwd):/app" \
  -w /app \
  rustow-e2e \
  bash -c "${command}"
~~~

## e2e

> Run the end-to-end scenario in a Void Linux podman container

~~~sh
$MASK container build
$MASK container run "bash tests/e2e.sh"
~~~

## check

> Run lint + tests

~~~sh
$MASK lint && $MASK test
~~~
